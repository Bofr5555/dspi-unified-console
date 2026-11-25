//! The shell, wired to a device.
//!
//! `Live` owns the shell, the device state and the session, and runs the
//! event loop: keys go to the shell, the shell's events become writes, meter
//! polls and notifications update the state, and the state is projected back
//! into the shell's model every frame. The command line and the palette live
//! here too, since they need the device.
//!
//! Screens (the detail region, the tool panels, Settings) come from a
//! [`Screens`] factory so the phases that build them plug in without touching
//! the loop.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use dspi_cmd::{Candidate, Context};
use dspi_proto::dsp;
use dspi_proto::value::Value;
use dspi_session::ext_state::ExtState;
use dspi_session::notify::{EchoFilter, Notification};
use dspi_session::{Applied, DeviceState, Notifications, Outcome, Session, Source};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::actions;
use crate::graph::GraphCurve;
use crate::perf::Performance;
use crate::screens::{
    self, AutoEqPanel, CrossfeedPanel, InputPage, LevellerPanel, LoudnessPanel, MatrixPanel,
    MonitorPanel, OutputPage, Overview, PresetChoice, PresetMenu, PsybassPanel, Shared,
    SignalsPanel, SpectrumPanel, StatsPanel, SubharmPanel, UpmixerPanel, clipboard, panel, presets,
};
use crate::settings::{AppConfig, SettingsData, SettingsScreen};
// -- WP TM8 --
use crate::shell::connect::{self, Reconnect, Reconnector, Target};
use crate::shell::{
    ChannelItem, Screen, Selection, Shell, ShellEvent, ShellModel, Tool, VolumeMode,
};
use crate::theme::{ChannelRole, Glyphs, Theme};
use crate::widgets::text::truncate;
use crate::widgets::{Button, Dialog, DialogOutcome, PopupList};
// -- WP P0M -- the meters' hold and clip hold are the session's, shared with the core
use dspi_session::link_words;
use dspi_session::meters::Ballistics;

/// The connect animation's length.
const REVEAL: Duration = Duration::from_millis(400);
/// How long a remotely changed value takes to reach its new position.
const EASE: Duration = Duration::from_millis(120);

/// Makes the screens the shell shows. The default makes placeholders; later
/// phases replace it.
pub trait Screens {
    fn detail(&self, state: &DeviceState, selection: Selection) -> Box<dyn Screen>;
    fn tool(&self, state: &DeviceState, tool: Tool) -> Box<dyn Screen>;
    fn settings(&self, state: &DeviceState) -> Box<dyn Screen>;

    /// The application-side state the screens share, when the factory keeps
    /// one. The runner needs it for the sidebar actions that are not a screen:
    /// the preset menu, the channel clipboard, and mirroring onto a linked
    /// input pair.
    fn shared(&self) -> Option<Shared> {
        None
    }

    /// The app-side settings file, which Settings > Graphing writes and the
    /// graph reads on start.
    fn config(&self) -> AppConfig {
        AppConfig::default()
    }

    // -- WP TM18 --
    /// Change the app-side settings file from outside Settings, for the
    /// choices the Console remembers where they are made.
    fn update_config(&self, _change: &mut dyn FnMut(&mut AppConfig)) -> std::io::Result<()> {
        Ok(())
    }

    /// Read what Settings needs that `DeviceState` does not carry. Called just
    /// before the page opens, so a change made elsewhere is on screen.
    fn refresh_settings(&self, _session: &mut Session) {}

    // -- WP PGTM --
    /// The open device's protocol for Settings > About, as the embedder read
    /// it at connect; `None` clears it. Kept until the next connection.
    fn set_protocol(&self, _protocol: Option<crate::settings::about::DeviceProtocolView>) {}
}

/// The Console's screens: the dashboard, the input and output pages, every
/// tool panel and Settings.
///
/// Every screen it makes shares one [`Shared`] handle, which is where the
/// linked pairs, the preset names, the channel clipboard and the spectrum
/// analyser's engine live.
pub struct ConsoleScreens {
    pub shared: Shared,
    /// What Settings reads that the bulk packet does not carry, refreshed the
    /// moment before the page opens.
    pub settings: std::rc::Rc<std::cell::RefCell<SettingsData>>,
    /// The app-side settings file, read at start and replaced by every
    /// change Settings saves, so the page reopens on what was last chosen.
    pub config: std::rc::Rc<std::cell::RefCell<AppConfig>>,
    // -- WP TM18 --
    /// Where that file is, when it is not the platform's own location.
    config_path: Option<std::path::PathBuf>,
}

// -- WP TM18 --
/// Change the settings, then write them out to `path` or the platform's own
/// location.
fn update_config(
    config: &std::cell::RefCell<AppConfig>,
    path: Option<&std::path::Path>,
    change: &mut dyn FnMut(&mut AppConfig),
) -> std::io::Result<()> {
    let mut c = config.borrow_mut();
    change(&mut c);
    match path {
        Some(p) => c.save_to(p),
        None => c.save().map(|_| ()),
    }
}

impl Default for ConsoleScreens {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleScreens {
    pub fn new() -> Self {
        Self::with_config(AppConfig::load(), None)
    }

    // -- WP TM18 --
    /// The same, with the settings file at `path` rather than the platform's
    /// own location.
    pub fn with_config_path(path: std::path::PathBuf) -> Self {
        Self::with_config(AppConfig::load_from(&path), Some(path))
    }

    fn with_config(config: AppConfig, config_path: Option<std::path::PathBuf>) -> Self {
        let shared = screens::shared();
        // The analyser's settings live with the engine, so a change on the
        // Settings page reaches it without a restart.
        shared
            .borrow_mut()
            .spectrum
            .adopt_settings(&config.spectrum);
        Self {
            shared,
            settings: std::rc::Rc::new(std::cell::RefCell::new(SettingsData::default())),
            config: std::rc::Rc::new(std::cell::RefCell::new(config)),
            config_path,
        }
    }
}

impl Screens for ConsoleScreens {
    fn detail(&self, state: &DeviceState, selection: Selection) -> Box<dyn Screen> {
        match selection {
            Selection::Overview => Box::new(Overview::new(self.shared.clone())),
            Selection::Input(i) => Box::new(InputPage::new(i, self.shared.clone(), state)),
            Selection::Output(o) => Box::new(OutputPage::new(o, self.shared.clone(), state)),
        }
    }

    fn tool(&self, state: &DeviceState, tool: Tool) -> Box<dyn Screen> {
        match tool {
            Tool::Matrix => Box::new(MatrixPanel::new(self.shared.clone())),
            Tool::Crossfeed => Box::new(CrossfeedPanel::new()),
            Tool::Loudness => Box::new(LoudnessPanel::new()),
            Tool::Leveller => Box::new(LevellerPanel::new()),
            Tool::Psybass => Box::new(PsybassPanel::new()),
            Tool::Upmixer => Box::new(UpmixerPanel::new()),
            Tool::Signals => Box::new(SignalsPanel::new()),
            Tool::Stats => Box::new(StatsPanel::new(self.shared.clone())),
            Tool::Monitor => Box::new(MonitorPanel::new(self.shared.clone())),
            Tool::AutoEq => Box::new(AutoEqPanel::new(self.shared.clone())),
            // -- WP TM18 --
            Tool::Tube => {
                let config = self.config.clone();
                let path = self.config_path.clone();
                let view = config.borrow().tube.view;
                Box::new(
                    crate::screens::TubePanel::with_view(view).on_view(move |view| {
                        update_config(&config, path.as_deref(), &mut |c| c.tube.view = view)
                    }),
                )
            }
            Tool::Subharm => Box::new(SubharmPanel::new()),
            Tool::Spectrum => Box::new(SpectrumPanel::open(self.shared.clone(), state)),
        }
    }

    fn settings(&self, state: &DeviceState) -> Box<dyn Screen> {
        let mut config = self.config.borrow().clone();
        config.spectrum = self.shared.borrow().spectrum.settings.clone();
        let shared = self.shared.clone();
        let kept = self.config.clone();
        let screen = SettingsScreen::new(state, self.settings.borrow().clone(), config).on_config(
            move |c: &AppConfig| {
                shared.borrow_mut().spectrum.adopt_settings(&c.spectrum);
                *kept.borrow_mut() = c.clone();
            },
        );
        // -- WP TM18 --
        Box::new(match &self.config_path {
            Some(p) => screen.config_path(p.clone()),
            None => screen,
        })
    }

    fn shared(&self) -> Option<Shared> {
        Some(self.shared.clone())
    }

    fn config(&self) -> AppConfig {
        self.config.borrow().clone()
    }

    // -- WP TM18 --
    fn update_config(&self, change: &mut dyn FnMut(&mut AppConfig)) -> std::io::Result<()> {
        update_config(&self.config, self.config_path.as_deref(), change)
    }

    fn refresh_settings(&self, session: &mut Session) {
        let mut data = SettingsData::read(session);
        data.rta = self.shared.borrow().spectrum.engine.caps().copied();
        // -- WP PGTM -- read once per connection by the embedder, not here
        data.protocol = self.settings.borrow().protocol.clone();
        *self.settings.borrow_mut() = data;
    }

    // -- WP PGTM --
    fn set_protocol(&self, protocol: Option<crate::settings::about::DeviceProtocolView>) {
        self.settings.borrow_mut().protocol = protocol;
    }
}

/// The verbs the interface answers itself: the Console's File, Tools and
/// AutoEQ menus.
///
/// They are not parameters, so they are not in the shared grammar and a shell
/// one-shot cannot run them; they need an interface with dialogs in it. They
/// are still typed and completed like everything else, because a person who
/// has learned `:` should not have to learn a second way to reach half of what
/// the program does.
pub const APP_VERBS: &[(&str, &str)] = &[
    ("import", "Import Filters..."),
    ("export", "Export Filters..."),
    ("import-config", "Import Device Configuration..."),
    ("export-config", "Export Device Configuration..."),
    ("autoeq", "AutoEQ: browse profiles, or `autoeq update`"),
    ("save-master", "Save Master Volume"),
    ("save-output-config", "Save Output Configuration"),
    ("commit", "Commit Parameters..."),
    ("revert", "Revert to Saved..."),
    ("factory-reset", "Factory Reset..."),
    ("bootloader", "Reboot into Bootloader..."),
    ("device", "Device picker"),
    ("reconnect", "Reconnect to the device"),
    ("clear-favourites", "AutoEQ: clear favourites"),
    ("tube", "Tube Modeller"),
    ("subharm", "Subharmonic Synthesizer"),
    // -- WP TMCOREF --
    ("unlock", "Unlock with the installer's PIN"),
    // -- WP UTM -- (UXP A13)
    ("help", "The keys here and everywhere (?)"),
];

// -- WP UTM -- (UXP A13, C10)
/// The shell's own verbs the `:` line runs too, on this interface's device,
/// instead of telling the person to leave and run them (UXP C10). The
/// embedder runs them ([`Live::set_shell_verbs`]): they are the binary's.
pub const SHELL_VERBS: &[(&str, &str)] = &[
    (
        "snapshot",
        "Snapshots: save NAME, list, recall ID, diff ID, delete ID",
    ),
    ("ab", "A/B: [A] B, two snapshots or the device now and one"),
    ("diff", "What differs from the saved preset or a snapshot"),
    (
        "lock",
        "The installer lock: status, unlock, set-pin, clear-pin",
    ),
    ("limits", "The installer's limits: show, set FILE, clear"),
    ("status", "The device, its link and its lock"),
    ("measure", "Measurement documents: docs, show ID, export ID"),
];

// -- WP UTM --
/// The `measure` words the `:` line runs: the documents only. Measuring and
/// the auto-tune stay one-shot commands (UXP A13).
const MEASURE_HERE: &[&str] = &["docs", "show", "export"];

// -- WP UTM --
/// Whether `verb` needs more words before it can run, so the palette puts it
/// on the `:` line rather than running it.
pub fn verb_takes_words(verb: &str) -> bool {
    matches!(verb, "snapshot" | "ab" | "lock" | "limits" | "measure")
}

// -- WP UTM --
/// Runs one of [`SHELL_VERBS`] (its words, the verb first) on the device of
/// `target`, with the interface's terminal handed back for the time it runs:
/// its output and its questions are the shell's. Answers the exit code.
pub type ShellVerbs = dyn Fn(&[String], Option<&Target>) -> u8;

/// The sub meter's poll period: the Console's 10 Hz
/// (`SubharmonicSynthView.swift:101`), and the brief's ceiling.
const SUBHARM_METER_PERIOD: Duration = Duration::from_millis(100);

/// What the runner remembers between the Subharmonic Synthesizer panel's
/// reads; see [`Live::poll_subharm`].
#[derive(Debug)]
struct SubharmPoll {
    open: bool,
    seen: Option<dspi_session::state::Subharm>,
    solo_at: Instant,
    meter_at: Instant,
}

impl Default for SubharmPoll {
    fn default() -> Self {
        let long_ago = Instant::now() - Duration::from_secs(2);
        Self {
            open: false,
            seen: None,
            solo_at: long_ago,
            meter_at: long_ago,
        }
    }
}

/// The `:` line and the `Ctrl-P` palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub palette: bool,
    pub input: String,
    pub candidates: Vec<Candidate>,
    pub index: usize,
    // -- WP UTM --
    /// The palette's finds, one per candidate (empty on the `:` line).
    pub hits: Vec<crate::search::Hit>,
}

impl Prompt {
    pub fn new(palette: bool, ctx: &Context) -> Self {
        let mut p = Self {
            palette,
            input: String::new(),
            candidates: Vec::new(),
            index: 0,
            hits: Vec::new(),
        };
        p.refresh(ctx);
        p
    }

    pub fn refresh(&mut self, ctx: &Context) {
        // -- WP UTM -- (UXP A13) the palette searches words; the `:` line completes
        if self.palette {
            self.hits = crate::search::search(&self.input, ctx);
            self.candidates = self
                .hits
                .iter()
                .map(|h| Candidate {
                    value: h.value.clone(),
                    detail: h.detail.clone(),
                    kind: match h.rank {
                        crate::search::Rank::Parameter => {
                            dspi_cmd::complete::CandidateKind::Parameter
                        }
                        _ => dspi_cmd::complete::CandidateKind::Verb,
                    },
                })
                .collect();
            if self.hits.is_empty() && !self.input.trim().is_empty() {
                self.candidates.push(Candidate {
                    value: String::new(),
                    detail: crate::search::no_match(&self.input),
                    kind: dspi_cmd::complete::CandidateKind::Hint,
                });
            }
            self.index = 0;
            return;
        }
        let ends_with_space = self.input.ends_with(' ');
        let mut tokens: Vec<&str> = self.input.split_whitespace().collect();
        let partial = if ends_with_space {
            ""
        } else {
            tokens.pop().unwrap_or("")
        };
        self.candidates = dspi_cmd::complete(&tokens, partial, ctx);
        // The interface's own verbs are offered alongside the grammar's, so
        // the palette is the whole program rather than the half of it that is
        // parameters.
        if tokens.is_empty() {
            let mine: Vec<Candidate> = APP_VERBS
                .iter()
                .chain(SHELL_VERBS) // -- WP UTM --
                .filter(|(name, _)| name.starts_with(partial))
                .map(|(name, detail)| Candidate {
                    value: (*name).to_string(),
                    detail: (*detail).to_string(),
                    kind: dspi_cmd::complete::CandidateKind::Verb,
                })
                .collect();
            // -- WP TM12 -- `autoeq` is a verb of both, and is listed once.
            self.candidates
                .retain(|c| !mine.iter().any(|m| m.value == c.value));
            self.candidates.splice(0..0, mine);
        }
        self.index = 0;
    }

    // -- WP UTM --
    /// What Enter does in the palette: the chosen find's action, or `None`
    /// to run the line as typed (a whole command typed into the palette
    /// still runs, as it always has).
    pub fn chosen(&self, ctx: &Context) -> Option<crate::search::Action> {
        let tokens = dspi_cmd::tokenize(&self.input);
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let command = refs.len() > 1
            && (dspi_cmd::parse(&refs, ctx).is_ok()
                || APP_VERBS
                    .iter()
                    .chain(SHELL_VERBS)
                    .any(|(n, _)| *n == refs[0]));
        if command {
            return None;
        }
        self.hits.get(self.index).map(|h| h.action.clone())
    }

    /// Returns `Some(line)` on Enter, `Some("")` on Escape.
    pub fn handle(&mut self, key: KeyEvent, ctx: &Context) -> Option<String> {
        match key.code {
            KeyCode::Esc => Some(String::new()),
            KeyCode::Enter => Some(std::mem::take(&mut self.input)),
            // -- WP UTM -- a find that goes to the `:` line goes there on Tab too
            KeyCode::Tab if self.palette => {
                if let Some(crate::search::Action::Complete(text)) =
                    self.hits.get(self.index).map(|h| h.action.clone())
                {
                    self.palette = false;
                    self.input = text;
                    self.refresh(ctx);
                }
                None
            }
            KeyCode::Tab => {
                if let Some(c) = self.candidates.get(self.index)
                    && !c.value.is_empty()
                {
                    let mut tokens: Vec<&str> = self.input.split_whitespace().collect();
                    if !self.input.ends_with(' ') {
                        tokens.pop();
                    }
                    let mut next = tokens.join(" ");
                    if !next.is_empty() {
                        next.push(' ');
                    }
                    next.push_str(&c.value);
                    next.push(' ');
                    self.input = next;
                    self.refresh(ctx);
                }
                None
            }
            KeyCode::Down => {
                if !self.candidates.is_empty() {
                    self.index = (self.index + 1) % self.candidates.len();
                }
                None
            }
            KeyCode::Up => {
                if !self.candidates.is_empty() {
                    self.index = self
                        .index
                        .checked_sub(1)
                        .unwrap_or(self.candidates.len() - 1);
                }
                None
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.refresh(ctx);
                None
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.refresh(ctx);
                None
            }
            _ => None,
        }
    }

    pub fn draw(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let height = (self.candidates.len().min(8) as u16 + 3).min(area.height);
        let rect = Rect::new(
            area.x + 2,
            area.y + area.height.saturating_sub(height + 2),
            area.width.saturating_sub(4),
            height,
        );
        Clear.render(rect, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(if theme.glyphs == Glyphs::Ascii {
                BorderType::Plain
            } else {
                BorderType::Rounded
            })
            .border_style(Style::default().fg(theme.accent))
            .title(if self.palette {
                " Search "
            } else {
                " Command "
            })
            .title_style(theme.title());
        let inner = block.inner(rect);
        block.render(rect, buf);
        let prompt = if self.palette { "› " } else { ":" };
        buf.set_string(inner.x, inner.y, prompt, Style::default().fg(theme.accent));
        let px = inner.x + prompt.chars().count() as u16;
        buf.set_string(
            px,
            inner.y,
            &self.input,
            Style::default().add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            px + self.input.chars().count() as u16,
            inner.y,
            "▏",
            Style::default().fg(theme.accent),
        );
        for (i, c) in self
            .candidates
            .iter()
            .take(inner.height as usize - 1)
            .enumerate()
        {
            let y = inner.y + 1 + i as u16;
            let style = if i == self.index {
                theme.focused()
            } else {
                theme.value()
            };
            if c.value.is_empty() {
                buf.set_string(
                    inner.x,
                    y,
                    truncate(&format!("  {}", c.detail), inner.width as usize),
                    theme.label(),
                );
            } else {
                buf.set_string(
                    inner.x,
                    y,
                    format!("  {:<24}", truncate(&c.value, 24)),
                    style,
                );
                buf.set_string(
                    inner.x + 26,
                    y,
                    truncate(&c.detail, (inner.width as usize).saturating_sub(26)),
                    theme.label(),
                );
            }
        }
    }
}

// -- WP TM19 --
/// One write of a block that is confirmed as a whole, parsed before anything
/// is asked or sent.
#[derive(Debug, Clone, PartialEq)]
struct BlockLine {
    /// The line as the screen wrote it, which is what a report names.
    line: String,
    path: &'static str,
    indices: Vec<u8>,
    value: Value,
}

/// Whether a parameter's write is confirmed first, the way every destructive
/// button in the Console is.
// -- WP TM18 --
fn volume_mode_of(choice: crate::settings::config::VolumeChoice) -> VolumeMode {
    match choice {
        crate::settings::config::VolumeChoice::Master => VolumeMode::Master,
        crate::settings::config::VolumeChoice::User => VolumeMode::User,
    }
}

// -- WP TMCORE --
/// The undo step's name for a block of writes with no note of its own.
const GROUP_LABEL: &str = "Edit";

// -- WP TMCORE --
/// The undo step's name for a block of lines: its last `#` note, which says
/// what the gesture did, or [`GROUP_LABEL`].
fn group_label(lines: &str) -> String {
    lines
        .lines()
        .filter_map(|l| l.trim().strip_prefix('#'))
        .next_back()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| GROUP_LABEL.to_string())
}

fn needs_confirming(desc: &dspi_proto::registry::ParamDesc) -> bool {
    matches!(
        desc.hazard,
        dspi_proto::registry::Hazard::Irreversible | dspi_proto::registry::Hazard::Flash
    )
}

/// Which app-level dialog is up, so its outcome is routed here rather than
/// to a screen.
#[derive(Debug, Clone, PartialEq)]
enum AppDialog {
    /// Quit, or switch to `Some(slot)`, after saving or discarding.
    Unsaved {
        then: PendingAction,
    },
    Rename {
        channel: usize,
    },
    PresetList(Vec<Option<PresetChoice>>),
    /// The `Copy to...` submenu, with the slot each row stands for.
    PresetCopyTo(Vec<u8>),
    PresetRename(u8),
    // -- WP TMCOREF --
    /// The installer's PIN, to unlock the installer lock.
    Unlock,
    PresetClear(u8),
    PresetClearAll,
    /// The Console's Core 1 collision prompt; `index` is the output to enable
    /// once the other side has been let go.
    Core1 {
        index: u8,
    },
    /// A command that cannot be undone, typed on the `:` line.
    Hazard {
        path: String,
        indices: Vec<u8>,
        value: Value,
    },
    // -- WP TM19 --
    /// Several writes in one block, some of which cannot be undone: one
    /// confirmation for all of them, kept here until it is answered.
    HazardBlock {
        lines: Vec<BlockLine>,
        note: Option<String>,
    },
    /// The Source row's list.
    SourceList,
    /// A path to read or write, by what it is for.
    Path(FileAction),
    /// Which channels an import lands on, and what each row stands for.
    ImportChannels {
        file: Box<dspi_session::filterfile::FilterFile>,
        targets: Vec<actions::ImportTarget>,
    },
    /// The `.dspipreset` options checklist, holding the document it describes.
    ConfigOptions(Box<dspi_session::preset_file::PresetDocument>),
    /// A report with nothing left to decide.
    Report,
    Commit,
    Revert,
    FactoryReset,
    Bootloader,
    /// The wait after the reboot; it closes itself.
    BootWait,
    /// Which device to talk to, with the serial each row stands for.
    DevicePicker(Vec<String>),
    /// The AutoEQ Update Database menu, with the method each button stands
    /// for, and the confirm in front of the rebuild.
    AutoEqUpdate(Vec<AutoEqMethod>),
    AutoEqRebuild,
    AutoEqRebuildProgress,
}

/// One of the Update Database menu's methods. The menu's buttons are named
/// by these rather than by position, because "Reset to Built-in" is offered
/// only when there is a user copy to throw away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoEqMethod {
    Rebuild,
    Import,
    Reset,
}

/// Which file action a path dialog is collecting a path for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileAction {
    ImportFilters,
    ExportFilters,
    ImportConfig,
    ExportConfig,
    ImportAutoEqDatabase,
}

#[derive(Debug, Clone, PartialEq)]
enum PendingAction {
    Quit,
    LoadPreset(u8),
    /// Copy the active preset to another slot once the live state is clean.
    CopyTo(u8),
    /// Open another device, by serial.
    SwitchDevice(String),
}

/// How long the runner waits for a deferred preset load, save or factory
/// reset to finish before going on without it: the Console's wait for a
/// save to land (`waitForPresetActivation`, Commands.swift:4213).
const DEFERRED_TIMEOUT: Duration = Duration::from_millis(1500);

/// The pause given to a deferred action there is no way to watch: the
/// Console's wait after a load before it re-reads (Commands.swift:4196).
const DEFERRED_SETTLE: Duration = Duration::from_millis(100);

/// What one batch of notifications asks of the runner.
#[derive(Debug, Default)]
struct NoteEffects {
    /// The shadow is out of date: read the whole packet again.
    reread: bool,
    /// The slot a `PRESET_LOADED` named.
    loaded: Option<u8>,
    /// A `BULK_INVALIDATED` from a preset operation.
    preset_bulk: bool,
    /// The last section someone else changed, and how they did, for the echo
    /// line.
    changed_elsewhere: Option<(&'static str, &'static str)>,
}

/// The device's active preset slot (`REQ_PRESET_GET_ACTIVE`,
/// vendor_commands.c:2814-2818), or `None` when it cannot be asked.
fn read_active_slot(session: &mut Session) -> Option<u8> {
    session
        .with_transport(|t| {
            t.control_in(dspi_proto::generated::opcodes::REQ_PRESET_GET_ACTIVE, 0, 1)
        })
        .ok()
        .and_then(|b| b.first().copied())
        .filter(|&s| (s as usize) < presets::SLOTS)
}

/// The shortest gap between two writes of one value while a key is held: the
/// Console's cap on drag traffic, 30 writes a second
/// (`PeqGraphEditor.swift`, Console 9c33a44).
pub const WRITE_INTERVAL: Duration = Duration::from_millis(33);

/// Holds back the writes a held arrow key produces.
///
/// Every write here is a round trip followed by a re-read (DESIGN 11), so a
/// key repeating faster than the device answers would leave keys waiting in
/// the terminal and the value moving on after the key is let go. Each value
/// (a parameter at its indices, or a whole band) is written at most once per
/// [`WRITE_INTERVAL`]; a newer value inside the interval replaces the one
/// held, and the held one goes when the interval ends, so the last value
/// always reaches the device.
#[derive(Debug, Default)]
struct WriteGate {
    /// When each value was last written.
    sent: std::collections::HashMap<String, Instant>,
    /// The newest line for a value inside its interval (with a band's
    /// bypass flag when it has one), and when it may go.
    held: Vec<(String, String, Instant)>,
}

pub struct Live {
    pub shell: Shell,
    pub state: DeviceState,
    pub perf: Performance,
    pub ctx: Context,
    pub prompt: Option<Prompt>,
    dialog: Option<(AppDialog, Dialog)>,
    popup: Option<(AppDialog, PopupList)>,
    status_until: Option<Instant>,
    /// Peak hold, clip hold and peak maximum (`dspi_session::meters`).
    meters: Ballistics,
    /// Whether a linked partner's curve is drawn under the selected one;
    /// `.` toggles it (DESIGN 12.2).
    partner_shown: bool,
    screens: Box<dyn Screens>,
    /// The state the screens share, when the factory keeps one.
    shared: Shared,
    pub should_quit: bool,
    /// The volume the slider is drawn at, easing toward the device's value
    /// when someone else moved it.
    shown_volume: Option<f64>,
    ease_from: Option<(Instant, f64)>,
    /// The notification reader, re-armed on every device switch.
    notes: Option<Notifications>,
    /// The link's kind, and our session id on a shared one: whose a change is
    /// depends on them (PLAN §1.2). Read when the notifications are armed.
    link_kind: dspi_transport::TransportKind,
    link_session: Option<u16>,
    /// Recognises the echo of a bulk write of ours. The terminal sends none
    /// today (`0xA1`/`0xA3` have no caller here), so nothing is ever
    /// outstanding and every invalidation reads as foreign, on either mode of
    /// firmware; a bulk write added here must bracket itself with
    /// `bulk_write_started`/`bulk_write_finished`.
    echo: EchoFilter,
    /// Where this device was found, when it can be found again. `None` for
    /// the built-in fixture.
    target: Option<Target>,
    /// Getting the device back once it is lost; `None` once that needs a
    /// person (or without a target).
    reconnect: Option<Reconnector>,
    /// The lost device's transport has been let go.
    offline: bool,
    last_screen_poll: Instant,
    /// The channel the pop-out graph is pinned to when Graphing says it does
    /// not follow the selection.
    popout_pinned: Option<usize>,
    devices_checked: Option<Instant>,
    /// When the upmixer's telemetry was last read. It is not a notification,
    /// so the only way to move the gauges is to ask, and once a second is
    /// enough for a meter a person is watching.
    last_upmix_poll: Instant,
    /// The Subharmonic Synthesizer panel's runtime reads: whether it was on
    /// screen last tick, the section its headroom was last read for, and when
    /// solo and the meters were last read.
    subharm: SubharmPoll,
    /// When the Stats panel's diagnostics were last read, on the Console's own
    /// two-second cadence.
    last_stats_poll: Instant,
    /// When the notification log started, which is what its time column counts
    /// from.
    started: Instant,
    /// The bootloader handoff, while it is running.
    boot: Option<actions::BootloaderWatch>,
    /// An AutoEQ rebuild, while it is running.
    rebuild: Option<dspi_session::autoeq::RebuildHandle>,
    // -- WP TM12 --
    /// What a rebuild fetches with: HTTPS, unless a test scripts it.
    autoeq_fetch: Box<dyn Fn() -> Box<dyn dspi_session::autoeq::Fetch>>,
    /// The serial of a device to open in place of this one. The event loop
    /// owns the transport, so the picker asks and the loop does it.
    pub switch_to: Option<String>,
    /// A key is waiting: the loop sets this before a tick so the analyser
    /// sits that tick out and the key's write goes first.
    pub input_pending: bool,
    /// The Graphing settings the graph was last built from, so a change
    /// saved in Settings reaches the graph without a restart.
    graphing: crate::settings::config::Graphing,
    // -- WP TM18 --
    /// The volume mode last read from the settings, to notice a new one.
    volume_choice: crate::settings::config::VolumeChoice,
    gate: WriteGate,
    /// A stopped clock for the write gate, so a test can hold a key at a
    /// chosen rate.
    clock: Option<Instant>,
    // -- WP TMCORE --
    /// How a device is opened again (a switch, a reconnect) when the
    /// embedder opens it otherwise than [`Target::connect`]: the Terminal
    /// puts it on a console core and routes the session's writes there.
    opener: Option<std::sync::Arc<connect::TargetOpener>>,
    // -- WP UTM --
    /// Runs the shell's verbs on this device; `None`: the `:` line says they
    /// are the shell's.
    shell_verbs: Option<std::rc::Rc<ShellVerbs>>,
    /// A shell verb's words, for the run loop to run with the terminal
    /// handed back.
    pub shell_run: Option<Vec<String>>,
}

impl Live {
    pub fn new(
        state: DeviceState,
        theme: Theme,
        perf: Performance,
        screens: Box<dyn Screens>,
    ) -> Self {
        let caps = &state.caps;
        let ctx = Context {
            channel_slugs: caps.channels.iter().map(|c| c.slug.clone()).collect(),
            num_inputs: caps.num_inputs,
            num_outputs: caps.num_outputs,
            max_bands: caps.max_bands,
        };
        let config = screens.config();
        let mut model = ShellModel::empty();
        model.selection = Selection::Overview;
        model.graph = crate::graph::GraphSettings::from_config(&config.graphing);
        model.volume_mode = volume_mode_of(config.volume.mode);
        let detail = screens.detail(&state, Selection::Overview);
        let shared = screens.shared().unwrap_or_else(screens::shared);
        let shell = Shell::new(model, theme, detail);
        let mut live = Self {
            shell,
            state,
            perf,
            ctx,
            prompt: None,
            dialog: None,
            popup: None,
            status_until: None,
            meters: Ballistics::default(),
            partner_shown: true,
            screens,
            shared,
            should_quit: false,
            shown_volume: None,
            ease_from: None,
            notes: None,
            last_screen_poll: Instant::now(),
            popout_pinned: None,
            devices_checked: None,
            link_kind: dspi_transport::TransportCaps::USB.kind,
            link_session: None,
            echo: EchoFilter::default(),
            target: None,
            reconnect: None,
            offline: false,
            last_upmix_poll: Instant::now() - Duration::from_secs(2),
            subharm: SubharmPoll::default(),
            last_stats_poll: Instant::now() - Duration::from_secs(3),
            started: Instant::now(),
            boot: None,
            rebuild: None,
            // -- WP TM12 --
            autoeq_fetch: Box::new(|| Box::new(crate::autoeq_fetch::HttpFetch::new())),
            switch_to: None,
            input_pending: false,
            graphing: config.graphing.clone(),
            volume_choice: config.volume.mode,
            gate: WriteGate::default(),
            clock: None,
            // -- WP TMCORE --
            opener: None,
            shell_verbs: None, // -- WP UTM --
            shell_run: None,
        };
        live.sync_model();
        live
    }

    // -- WP UTM --
    /// Run the shell's verbs ([`SHELL_VERBS`]) on the `:` line with `run`.
    pub fn set_shell_verbs(&mut self, run: std::rc::Rc<ShellVerbs>) {
        self.shell_verbs = Some(run);
    }

    // -- WP UTM --
    /// Run the shell verb [`Live::shell_run`] holds, with the terminal already
    /// handed back by the run loop; then re-read what it may have changed.
    pub fn run_shell_verb(&mut self, session: &mut Session, words: &[String]) -> u8 {
        let Some(run) = self.shell_verbs.clone() else {
            return 0;
        };
        let code = run(words, self.target.as_ref());
        self.refresh(session);
        self.refresh_presets(session);
        let line = words.join(" ");
        self.note(if code == 0 {
            format!("Ran :{line}")
        } else {
            format!(":{line} ended with code {code}")
        });
        code
    }

    // -- WP TMCORE --
    /// Open devices with `open` from now on (a switch, a reconnect), in
    /// place of [`Target::connect`]. Call before [`Live::set_connection`].
    pub fn set_opener(&mut self, open: std::sync::Arc<connect::TargetOpener>) {
        self.opener = Some(open);
    }

    // -- WP TMCORE --
    /// The way back to `target`: through the embedder's opener when it set one.
    fn reconnector(&self, target: Target) -> Reconnector {
        match &self.opener {
            Some(open) => Reconnector::with_open(target, std::sync::Arc::clone(open)),
            None => Reconnector::new(target),
        }
    }

    // -- WP TMCORE --
    /// Open `target` now, as a device switch does.
    fn open_target(&self, target: &Target) -> Result<connect::Connected, connect::OpenError> {
        match &self.opener {
            Some(open) => open(target, &connect::no_prompt),
            None => target.connect(&connect::no_prompt),
        }
    }

    /// Project the device state into the shell's model.
    pub fn sync_model(&mut self) {
        // -- WP TM8 --
        let lost = self.link_reconnecting();
        self.state.mirror_held = lost;
        self.shell.model.link_lost = lost;
        let s = &self.state;
        let caps = &s.caps;
        let t = &self.shell.theme;
        let m = &mut self.shell.model;
        let (ni, no) = (caps.num_inputs as usize, caps.num_outputs as usize);
        m.platform = format!("{:?}", caps.platform).to_uppercase();
        m.firmware = caps.firmware.clone();
        // By characters: the device chooses this text, and slicing it by bytes
        // can land inside one.
        let extra = caps.serial.chars().count().saturating_sub(8);
        m.serial_short = caps.serial.chars().skip(extra).collect();
        m.connected = s.connected;
        m.preset_label = match caps.active_preset {
            Some(p) => PresetMenu::slot_label(&self.shared.borrow(), p),
            None => "Empty".into(),
        };
        m.preset_dirty = s.has_unsaved_changes();

        let item =
            |ch: usize, role: ChannelRole, inactive: bool, meters: &Ballistics| ChannelItem {
                name: {
                    // -- BUG1 -- an output's label, short beside its OUTn: the user's name, else
                    // channel and band from the routing and the filters ("R·Lo")
                    let n = screens::channel_short_label(s, ch);
                    if n.is_empty() {
                        role.descriptor(no as u8)
                    } else {
                        n
                    }
                },
                descriptor: role.descriptor(no as u8),
                role,
                color: t.role_color(role),
                level: s.meters.peaks.get(ch).copied().unwrap_or(0.0),
                peak: meters.hold(ch),
                clipped: s.is_clipped(ch),
                inactive,
                index: ch as u8,
                unused: screens::channel_unused(s, ch), // -- WP UTM --
            };
        let active_inputs = s.meters.active_inputs as usize;
        m.inputs = (0..ni)
            .map(|i| {
                item(
                    i,
                    ChannelRole::Input(i as u8),
                    active_inputs > 0 && i >= active_inputs,
                    &self.meters,
                )
            })
            .collect();
        m.outputs = (0..no)
            .map(|o| {
                let out = s.output(o);
                let role = ChannelRole::of((ni + o) as u8, ni as u8, no as u8);
                item(ni + o, role, !out.enabled || out.mute, &self.meters)
            })
            .collect();
        // Without a device the sidebar shows no channel rows: they would only
        // describe the last device's layout (`ContentView.swift:438-446`).
        // -- WP TM8 -- But a hub being reconnected to keeps them.
        if !s.mirror_readable() {
            m.inputs.clear();
            m.outputs.clear();
        }

        let g = s.global();
        m.strip[1].state = Some(s.crossfeed().enabled);
        m.strip[2].state = Some(g.loudness_enabled);
        m.strip[3].state = Some(s.leveller().enabled);
        m.strip[4].state = Some(s.psybass().enabled);
        m.strip[7].state = Some(g.bypass);

        let source_supported = caps
            .features
            .iter()
            .any(|f| f.name == "spdif_multi_input" && f.present)
            || caps
                .features
                .iter()
                .any(|f| f.name == "i2s_input_channels" && f.present);
        m.source = if source_supported {
            let mut choices: Vec<String> = vec![
                "USB".into(),
                "S/PDIF".into(),
                "I2S".into(),
                "ADAT".into(),
                "S/PDIF 2".into(),
                "S/PDIF 3".into(),
                "S/PDIF 4".into(),
            ];
            let raw = s
                .input_config()
                .map(|c| c.input_source as usize)
                .unwrap_or(0);
            // -- WP SW1 (fixup sw1, F4) -- a unit on the audio link (7) shows
            // it; it is offered only there, as the Terminal does not probe
            // the link yet.
            if raw == usize::from(dspi_proto::ext::alink::INPUT_SOURCE_ALINK) {
                choices.push("Audio link".into());
            }
            let idx = raw.min(choices.len() - 1);
            Some((choices, idx))
        } else {
            None
        };
        let target = match m.volume_mode {
            VolumeMode::User => s.user_volume().0 as f64,
            VolumeMode::Master => s.master_volume_db() as f64,
        };
        // A value the device changed eases to its new place, so a knob turn
        // reads as motion rather than a jump. Our own writes snap.
        m.volume_db = match (self.ease_from, self.perf.animate) {
            (Some((since, from)), true) => {
                let t = (since.elapsed().as_secs_f64() / EASE.as_secs_f64()).min(1.0);
                if t >= 1.0 {
                    self.ease_from = None;
                }
                from + (target - from) * t
            }
            _ => target,
        };
        self.shown_volume = Some(target);
        m.cpu = (s.meters.cpu0, s.meters.cpu1);

        // Curves: the graphed channel's PEQ plus crossover bands, with output
        // gain folded in, and its linked partner's underneath in grey.
        let selected_index: Option<usize> = match m.selection {
            Selection::Overview => None,
            Selection::Input(i) => Some(i),
            Selection::Output(o) => Some(ni + o),
        };
        // No curves without a device: the magnitudes are the last device's.
        // The graph keeps its grid (`GraphView.swift:212-216`).
        let graphed = self
            .popout_pinned
            .or(selected_index)
            // -- WP TM8 --
            .filter(|_| s.mirror_readable());
        let partner = graphed
            .filter(|_| self.partner_shown)
            .and_then(|ch| self.shared.borrow().linked_partner(ch, ni));
        let mut curves = Vec::with_capacity(2);
        for ch in partner.into_iter().chain(graphed) {
            let principal = Some(ch) == graphed;
            let mut bands: Vec<dsp::Band> = s
                .bands(ch as u8)
                .iter()
                .map(|p| dsp::Band {
                    filter_type: p.filter_type,
                    freq: p.freq,
                    q: p.q,
                    gain_db: p.gain_db,
                    bypass: p.bypass,
                })
                .collect();
            let gain = if ch >= ni {
                bands.extend(s.xover_bands(ch as u8).iter().map(|p| dsp::Band {
                    filter_type: p.filter_type,
                    freq: p.freq,
                    q: p.q,
                    gain_db: p.gain_db,
                    bypass: p.bypass,
                }));
                s.output(ch - ni).gain_db as f64
            } else {
                0.0
            };
            let role = ChannelRole::of(ch as u8, ni as u8, no as u8);
            curves.push(GraphCurve {
                descriptor: role.descriptor(no as u8),
                color: if principal { t.role_color(role) } else { t.dim },
                magnitude: reveal(dsp::curve(&bands, gain), self.started, self.perf.animate),
                phase: if principal && m.graph.show_phase {
                    Some(dsp::phase_curve(&bands))
                } else {
                    None
                },
                selected: principal,
            });
        }
        m.curves = curves;
        m.graph_channel = graphed.map(|ch| screens::channel_label(s, ch));
        self.shared.borrow_mut().graph = m.graph.clone();
        // -- WP TM5 --
        m.spectrum = screens::spectrum::overlay_picture(&self.shared, s, t, Instant::now());
        // -- WP TM6 --
        m.bars = screens::spectrum::bars_picture(&self.shared, s, t, Instant::now());
    }

    // -- WP TM12 --
    /// Fetch with this instead of HTTPS on the next AutoEQ rebuild.
    pub fn set_autoeq_fetch(
        &mut self,
        fetch: impl Fn() -> Box<dyn dspi_session::autoeq::Fetch> + 'static,
    ) {
        self.autoeq_fetch = Box::new(fetch);
    }

    fn note(&mut self, text: impl Into<String>) {
        self.shell.model.status = Some(text.into());
        self.status_until = Some(Instant::now() + Duration::from_secs(3));
    }

    fn echo(&mut self, text: impl Into<String>) {
        self.shell.model.echo = text.into();
    }

    /// Echo a write the way the grammar types it, so what the echo line shows
    /// can be typed back: output and slot indices are 0-based there.
    fn echo_set(&mut self, path: &str, indices: &[u8], value: Value) {
        if let Some(d) = dspi_proto::registry::by_path(path) {
            let cmd = dspi_cmd::Command::Set {
                path: d.path,
                indices: indices.to_vec(),
                value,
            };
            self.echo(dspi_cmd::format(&cmd, &self.ctx));
        }
    }

    /// Re-read the bulk packet and refresh the model. False when the read
    /// failed, which leaves the old shadow in place.
    pub fn refresh(&mut self, session: &mut Session) -> bool {
        let ok = match session.snapshot() {
            Ok(b) => {
                self.state.replace_bulk(b);
                true
            }
            Err(e) => {
                self.note(e.to_string());
                false
            }
        };
        self.sync_model();
        ok
    }

    /// Write one registry parameter, echoing the canonical command.
    pub fn set(&mut self, session: &mut Session, path: &str, indices: &[u8], value: Value) {
        let Some(desc) = dspi_proto::registry::by_path(path) else {
            self.note(format!("unknown parameter {path}"));
            return;
        };
        if needs_confirming(desc) {
            self.dialog = Some((
                AppDialog::Hazard {
                    path: path.to_string(),
                    indices: indices.to_vec(),
                    value: value.clone(),
                },
                Dialog::confirm(
                    "Are you sure?",
                    format!("`{path}`: {}. This cannot be undone.", desc.plain),
                    vec![Button::destructive("Continue"), Button::new("Cancel")],
                ),
            ));
            return;
        }
        self.write_set(session, path, indices, value);
    }

    /// Write one parameter that needs no further confirmation, leaving a note
    /// if it fails. True when the device took it.
    fn write_set(
        &mut self,
        session: &mut Session,
        path: &str,
        indices: &[u8],
        value: Value,
    ) -> bool {
        let Some(desc) = dspi_proto::registry::by_path(path) else {
            self.note(format!("unknown parameter {path}"));
            return false;
        };
        // Anything the device has said goes in first, so a preset loaded
        // elsewhere is the baseline before this write lands on it.
        self.absorb_pending_notes(session);
        // The deferred preset and reset actions have their own flows, which
        // wait for the device to finish before re-reading.
        match (path, indices.first()) {
            ("preset.load", Some(&slot)) => return self.load_preset(session, slot),
            ("preset.save", Some(&slot)) => {
                let saved = self.save_slot(session, slot);
                if saved {
                    if Some(slot) == self.state.caps.active_preset {
                        self.state.mark_saved();
                    }
                    self.echo_set(path, indices, value);
                    self.refresh_presets(session);
                } else {
                    self.note("Save Failed");
                }
                return saved;
            }
            ("dev.reset", _) => {
                self.factory_reset(session);
                return true;
            }
            _ => {}
        }
        // Enabling an output goes through the Core 1 interlock, which may
        // need the Console's confirmation before the other side is freed.
        if path == "out.enable"
            && let (Some(&index), Some(enable)) = (indices.first(), value.as_bool())
        {
            self.enable_output(session, index, enable);
            return true;
        }
        let cmd = dspi_cmd::Command::Set {
            path: desc.path,
            indices: indices.to_vec(),
            value: value.clone(),
        };
        // In INDEPENDENT mode a limiter write is output configuration: keep
        // what it replaces so Settings can offer Save and Revert for it.
        if path.starts_with("limit.") {
            self.state.begin_limiter_edit();
        }
        let mode = (path == "preset.iomode").then(|| value.as_u8()).flatten();
        let master_mode = (path == "vol.master.mode").then(|| value.as_u8()).flatten();
        match session.write(path, indices, value) {
            // A packet parameter has no scalar readback: the confirming read
            // asks for one byte of a structure, so a byte-for-byte comparison
            // is not evidence of anything. Take the device's word for it.
            Ok(Outcome::Rejected { .. })
                if matches!(desc.kind, dspi_proto::registry::Kind::Packet) =>
            {
                self.echo(dspi_cmd::format(&cmd, &self.ctx));
                self.refresh(session);
                true
            }
            Ok(Outcome::Rejected { actual, .. }) => {
                let shown = dspi_proto::registry::by_path(path)
                    .map(|d| display_value(d, &actual))
                    .unwrap_or_default();
                self.note(format!("{path} was not applied; device kept {shown}"));
                false
            }
            Ok(_) => {
                self.echo(dspi_cmd::format(&cmd, &self.ctx));
                if path == "dev.save.io" {
                    self.state.limiter_saved();
                }
                if let Some(mode) = mode {
                    self.state.output_config_mode = mode;
                }
                if let Some(mode) = master_mode {
                    self.state.master_volume_mode = mode;
                }
                // Solo has no wire offset and no notification (config.h:201),
                // so the re-read that follows every write cannot bring it.
                if path == "sub.solo" {
                    let _ = self.state.refresh_subharm_solo(session);
                }
                self.refresh(session);
                true
            }
            Err(e) => {
                self.note_write_error(session, &e); // -- WP TMCOREF --
                false
            }
        }
    }

    /// Run one or more commands, one per line.
    ///
    /// A screen asks for several at once when one gesture is several writes:
    /// an edit mirrored onto a linked input pair, Clear All over a whole bank,
    /// a channel paste. The echo line ends up showing the last of them, which
    /// is the one the person's finger was on.
    /// A `#` line is not a command but the note to leave on the echo line once
    /// the block has run, which is how a screen that issues a hundred writes
    /// for one gesture says what it just did.
    ///
    /// A block of two or more commands with one that needs confirming is
    /// confirmed once, as a whole; see [`Self::confirm_block`].
    pub fn run_commands(&mut self, session: &mut Session, lines: &str) {
        // -- WP TM19 --
        let commands: Vec<&str> = lines
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        if commands.len() > 1 && commands.iter().any(|l| self.line_needs_confirming(l)) {
            let note = lines
                .lines()
                .filter_map(|l| l.trim().strip_prefix('#'))
                .next_back()
                .map(|n| n.trim().to_string());
            self.confirm_block(&commands, note);
            return;
        }
        // -- WP TMCORE -- (O6) one gesture of several writes is one undo step
        let group = (commands.len() > 1).then(|| group_label(lines));
        if let Some(label) = &group {
            session.begin_group(label);
        }
        let mut note = None;
        for line in lines.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match line.strip_prefix('#') {
                Some(text) => note = Some(text.trim().to_string()),
                None => self.run_command(session, line),
            }
        }
        if group.is_some() {
            session.end_group();
        }
        if let Some(note) = note {
            self.note(note);
        }
    }

    // -- WP TM19 --
    /// The registry write a line asks for, if that is all it is.
    fn parse_write(&self, line: &str) -> Option<BlockLine> {
        let tokens = dspi_cmd::tokenize(line);
        let first = tokens.first()?;
        if APP_VERBS.iter().any(|(n, _)| n == first) {
            return None;
        }
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match dspi_cmd::parse(&refs, &self.ctx).ok()? {
            dspi_cmd::Command::Set {
                path,
                indices,
                value,
            } => Some(BlockLine {
                line: line.to_string(),
                path,
                indices,
                value,
            }),
            _ => None,
        }
    }

    fn line_needs_confirming(&self, line: &str) -> bool {
        self.parse_write(line)
            .and_then(|w| dspi_proto::registry::by_path(w.path))
            .is_some_and(needs_confirming)
    }

    /// Ask once about a whole block: one answer covers every line, and they
    /// are then written in order (`run_block`).
    ///
    /// A line that is not a plain write has no result to report, so a block
    /// holding one is refused before anything is asked or written. No screen
    /// builds such a block; the check keeps a bad line from flashing half of it.
    fn confirm_block(&mut self, commands: &[&str], note: Option<String>) {
        let mut lines = Vec::with_capacity(commands.len());
        for line in commands {
            match self.parse_write(line) {
                Some(w) => lines.push(w),
                None => {
                    self.note(format!("`{line}` cannot be confirmed; nothing was written"));
                    return;
                }
            }
        }
        let mut body = Vec::new();
        let mut others = 0usize;
        for w in &lines {
            match dspi_proto::registry::by_path(w.path).filter(|d| needs_confirming(d)) {
                Some(d) => body.push(format!("`{}`: {}", w.path, d.plain)),
                None => others = others.saturating_add(1),
            }
        }
        if others > 0 {
            body.push(format!("and {others} more in the same step"));
        }
        body.push(String::new());
        body.push("This cannot be undone.".to_string());
        // The report kind keeps one line per write, where a confirm's body
        // would run them together; only its button row differs.
        let mut dialog = Dialog::report("Are you sure?", body);
        dialog.buttons = vec![Button::destructive("Continue"), Button::new("Cancel")];
        self.dialog = Some((AppDialog::HazardBlock { lines, note }, dialog));
    }

    /// Write a confirmed block in order, stopping at the first line the device
    /// does not take. Nothing is retried: the screen hears which lines were
    /// written, so the rest stay in its draft, and the person hears both.
    fn run_block(&mut self, session: &mut Session, lines: Vec<BlockLine>, note: Option<String>) {
        let mut written = Vec::new();
        let mut failure = None;
        // -- WP TMCORE -- (O6) the confirmed block is one undo step
        session.begin_group(note.as_deref().unwrap_or(GROUP_LABEL));
        let mut rest = lines.into_iter();
        for w in rest.by_ref() {
            if self.write_set(session, w.path, &w.indices, w.value) {
                written.push(w.line);
            } else {
                let why = self.shell.model.status.clone().unwrap_or_default();
                failure = Some((w.line, why));
                break;
            }
        }
        session.end_group();
        self.tell_written(session, &written);
        let Some((failed, why)) = failure else {
            if let Some(note) = note {
                self.note(note);
            }
            return;
        };
        let mut report = vec!["Written:".to_string()];
        report.extend(written.iter().map(|l| format!("  {l}")));
        if written.is_empty() {
            report.push("  nothing".to_string());
        }
        report.push("Not written:".to_string());
        report.push(format!("  {failed}"));
        report.push(format!("    {why}"));
        report.extend(rest.map(|w| format!("  {}", w.line)));
        report.push("Nothing was retried. What was not written is still unsaved.".to_string());
        self.dialog = Some((AppDialog::Report, Dialog::report("Stopped partway", report)));
    }

    /// Tell Settings which lines reached the device: only those count as saved.
    fn tell_written(&mut self, session: &mut Session, written: &[String]) {
        if written.is_empty() {
            return;
        }
        if let Some(s) = self.shell.settings.as_deref_mut() {
            s.lines_written(written, session, &self.state);
        }
    }

    fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    /// The value a line writes, when it is one a held key moves: a number
    /// set on a parameter, or a whole band. Toggles, choices and actions are
    /// never held back.
    fn gate_key(&self, line: &str) -> Option<String> {
        let tokens = dspi_cmd::tokenize(line);
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match dspi_cmd::parse(&refs, &self.ctx).ok()? {
            dspi_cmd::Command::Set { path, indices, .. } => {
                let d = dspi_proto::registry::by_path(path)?;
                matches!(
                    d.kind,
                    dspi_proto::registry::Kind::Float { .. }
                        | dspi_proto::registry::Kind::Int { .. }
                )
                .then(|| format!("{path} {indices:?}"))
            }
            dspi_cmd::Command::SetBand { channel, band, .. } => {
                Some(format!("eq {channel} {band}"))
            }
            _ => None,
        }
    }

    /// The band a line switches the bypass flag of, keyed as
    /// [`Self::gate_key`] keys the band's `eq` line.
    fn band_flag_key(&self, line: &str) -> Option<String> {
        let tokens = dspi_cmd::tokenize(line);
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match dspi_cmd::parse(&refs, &self.ctx).ok()? {
            dspi_cmd::Command::Set {
                path: "eq.bypass",
                indices,
                ..
            } => {
                let (&channel, &band) = (indices.first()?, indices.get(1)?);
                Some(format!("eq {channel} {band}"))
            }
            _ => None,
        }
    }

    /// Run the lines a screen asked for, holding back any value written less
    /// than [`WRITE_INTERVAL`] ago (see [`WriteGate`]).
    ///
    /// Order is kept. A band's `eq` line and the bypass flag after it are one
    /// unit, held or sent together: the device applies the packet's bypass
    /// byte (vendor_commands.c:545-550), which `eq` always sends clear, so a
    /// flag sent ahead of a held `eq` would be undone when the `eq` went. A
    /// line that is never held sends every held write first, so it cannot
    /// overtake one.
    fn run_screen_commands(&mut self, session: &mut Session, lines: &str) {
        // -- WP TM19 -- A line that needs confirming never waits and never
        // joins a held write in its block: the held writes go first, whatever
        // the answer turns out to be.
        if lines.lines().any(|l| self.line_needs_confirming(l.trim())) {
            self.flush_writes(session, true);
            self.run_commands(session, lines);
            return;
        }
        let now = self.now();
        let lines: Vec<&str> = lines
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let mut run: Vec<String> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            i += 1;
            if line.starts_with('#') {
                run.push(line.to_string());
                continue;
            }
            let Some(key) = self.gate_key(line) else {
                for (k, held, _) in std::mem::take(&mut self.gate.held) {
                    self.gate.sent.insert(k, now);
                    run.push(held);
                }
                run.push(line.to_string());
                continue;
            };
            let mut unit = line.to_string();
            while let Some(next) = lines.get(i)
                && self.band_flag_key(next).as_deref() == Some(key.as_str())
            {
                unit.push('\n');
                unit.push_str(next);
                i += 1;
            }
            self.gate.held.retain(|(k, _, _)| *k != key);
            match self.gate.sent.get(&key).map(|at| *at + WRITE_INTERVAL) {
                Some(due) if now < due => self.gate.held.push((key, unit, due)),
                _ => {
                    self.gate.sent.insert(key, now);
                    run.push(unit);
                }
            }
        }
        // A block whose every write is held has nothing to say yet.
        if run.iter().any(|l| !l.starts_with('#')) {
            self.run_commands(session, &run.join("\n"));
        }
    }

    /// Send every held write whose interval has ended; with `all`, every held
    /// write, for anything that must see the device settled first.
    pub fn flush_writes(&mut self, session: &mut Session, all: bool) {
        let now = self.now();
        let (due, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.gate.held)
            .into_iter()
            .partition(|(_, _, at)| all || *at <= now);
        self.gate.held = kept;
        for (key, line, _) in due {
            self.gate.sent.insert(key, now);
            self.run_commands(session, &line);
        }
        self.gate
            .sent
            .retain(|_, at| now.saturating_duration_since(*at) < WRITE_INTERVAL);
    }

    /// Leave the device as the person would expect to find it, before quitting
    /// or opening another one: every held write sent, and the sub solo off.
    /// Solo mutes the program on the masked outputs and is runtime state the
    /// device keeps until told (config.h:201), so the Console switches it off
    /// when its window closes; quitting closes the panel too.
    pub fn release(&mut self, session: &mut Session) {
        self.flush_writes(session, true);
        if self.state.subharm_solo == Some(true) {
            self.set(session, "sub.solo", &[], Value::Bool(false));
        }
    }

    fn enable_output(&mut self, session: &mut Session, index: u8, enable: bool) {
        match session.enable_output(index, enable) {
            Ok(dspi_session::EnableOutcome::Done) => {
                self.echo_set("out.enable", &[index], Value::Bool(enable));
                self.refresh(session);
            }
            Ok(dspi_session::EnableOutcome::NeedsConfirm(c)) => {
                self.dialog = Some((
                    AppDialog::Core1 { index },
                    Dialog::confirm(
                        c.title,
                        c.body,
                        vec![Button::destructive(c.confirm), Button::new("Cancel")],
                    ),
                ));
            }
            Ok(dspi_session::EnableOutcome::Rejected) => {
                self.note("The device kept the output as it was")
            }
            Err(e) => self.note(e.to_string()),
        }
    }

    fn undo(&mut self, session: &mut Session, redo: bool) {
        self.flush_writes(session, true);
        // A reversal can be a limiter write, which in INDEPENDENT mode is
        // output configuration: keep the baseline it moves away from, as
        // `set` does, or undoing past a save would read as saved. Taken when
        // the limiters are clean, the baseline is what they are now, so doing
        // this for a reversal that is not a limiter write changes nothing.
        self.state.begin_limiter_edit();
        let result = if redo { session.redo() } else { session.undo() };
        match result {
            Ok(Some(u)) => {
                match u.command {
                    Some(c) => self.echo(format!("{} {c}", if redo { "redo:" } else { "undo:" })),
                    None => self.note("Nothing reversible to undo"),
                }
                if !u.skipped.is_empty() {
                    self.note(format!("Skipped {}", u.skipped.join("; ")));
                }
                self.refresh(session);
            }
            Ok(None) => self.note(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            }),
            Err(e) => self.note(e.to_string()),
        }
    }

    /// Run a typed command against the device.
    pub fn run_command(&mut self, session: &mut Session, line: &str) {
        let tokens = dspi_cmd::tokenize(line);
        if let Some(first) = tokens.first()
            && APP_VERBS.iter().any(|(n, _)| n == first)
        {
            let args: Vec<&str> = tokens[1..].iter().map(String::as_str).collect();
            self.app_verb(session, first, &args);
            return;
        }
        // -- WP UTM -- (UXP A13, C10)
        if let Some(first) = tokens.first()
            && SHELL_VERBS.iter().any(|(n, _)| n == first)
        {
            self.shell_verb(tokens);
            return;
        }
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let cmd = match dspi_cmd::parse(&refs, &self.ctx) {
            Ok(c) => c,
            // -- WP UTM -- (UXP C10) the way on is in here, not in the shell
            Err(dspi_cmd::ParseError::Unknown(word)) => {
                self.note(format!(
                    "No parameter or command called `{word}` · Ctrl-P searches, ? lists the keys"
                ));
                return;
            }
            Err(e) => {
                self.note(e.to_string());
                return;
            }
        };
        match cmd {
            dspi_cmd::Command::Set {
                path,
                ref indices,
                ref value,
            } => {
                self.set(session, path, indices, value.clone());
            }
            dspi_cmd::Command::Get { path, ref indices } => match session.read_whole(path, indices)
            {
                // A status or packet row, read at its own length.
                Ok(Some(r)) => self.note(format!("{path} = {}", r.one_line())),
                Ok(None) => match session.read(path, indices) {
                    Ok(v) => {
                        let shown = dspi_proto::registry::by_path(path)
                            .map(|d| display_value(d, &v))
                            .unwrap_or_default();
                        self.note(format!("{path} = {shown}"));
                    }
                    Err(e) => self.note(e.to_string()),
                },
                Err(e) => self.note(e.to_string()),
            },
            dspi_cmd::Command::SetBand {
                channel,
                band,
                filter_type,
                freq,
                q,
                gain,
                qp,
            } => {
                let packet = dspi_proto::value::EqParamPacket {
                    channel,
                    band,
                    filter_type: dspi_proto::FilterType::from_raw(filter_type),
                    bypass: false,
                    freq,
                    q,
                    gain_db: gain,
                    qp,
                };
                self.absorb_pending_notes(session);
                match session.write_band(&packet) {
                    Ok(Outcome::Rejected { .. }) => self.note("the band was not applied as sent"),
                    Ok(_) => {
                        self.echo(dspi_cmd::format(&cmd, &self.ctx));
                        self.refresh(session);
                    }
                    Err(e) => self.note_write_error(session, &e), // -- WP TMCOREF --
                }
            }
            dspi_cmd::Command::Verb { name, .. } => match name.as_str() {
                "undo" => self.undo(session, false),
                "redo" => self.undo(session, true),
                "reconnect" => {
                    // The Console's right-click on the device name.
                    self.switch_to = Some(self.state.caps.serial.clone());
                }
                "clear-favourites" => match dspi_session::autoeq::save_favourites(&[]) {
                    Ok(()) => self.note("Favourites cleared"),
                    Err(e) => self.note(e.to_string()),
                },
                other => self.note(format!("`{other}` only works from the shell")),
            },
        }
    }

    // -- WP UTM --
    /// One of the shell's verbs: handed to the run loop, which gives the
    /// terminal back while it runs. `measure` runs its documents only.
    fn shell_verb(&mut self, tokens: Vec<String>) {
        if self.shell_verbs.is_none() {
            self.note(format!("`{}` only works from the shell", tokens[0]));
            return;
        }
        if tokens[0] == "measure"
            && !tokens
                .get(1)
                .is_some_and(|w| MEASURE_HERE.contains(&w.as_str()))
        {
            self.note(format!(
                ":measure takes {} here; measuring and the auto-tune run as `{} measure`",
                MEASURE_HERE.join(", "),
                dspi_cmd::COMMAND
            ));
            return;
        }
        if tokens.len() == 1 && verb_takes_words(&tokens[0]) && tokens[0] != "ab" {
            let words = SHELL_VERBS
                .iter()
                .find(|(n, _)| *n == tokens[0])
                .map_or("", |(_, d)| d);
            self.note(format!(":{} takes words: {words}", tokens[0]));
            return;
        }
        self.shell_run = Some(tokens);
    }

    // -- WP UTM --
    /// What a palette find does when it is chosen.
    fn palette_action(&mut self, session: &mut Session, action: crate::search::Action) {
        use crate::search::Action;
        match action {
            Action::Event(ev) => self.handle_event(session, ev),
            Action::Line(line) => {
                self.flush_writes(session, true);
                self.run_command(session, &line);
            }
            Action::Complete(text) => {
                let mut p = Prompt::new(false, &self.ctx);
                p.input = text;
                p.refresh(&self.ctx);
                self.prompt = Some(p);
            }
            Action::Help => self.shell.help = true,
            Action::Volume => {
                self.shell.focus = crate::shell::Focus::Footer(crate::shell::FooterRow::Volume)
            }
            Action::Crossover => self.open_crossover(session),
        }
    }

    // -- WP UTM --
    /// The selected output's page on its XO tab; the first output in use
    /// when no output is selected.
    fn open_crossover(&mut self, session: &mut Session) {
        let outputs = self.shell.model.outputs.len();
        if outputs == 0 {
            self.note("No outputs: no device is open");
            return;
        }
        let o = match self.shell.model.selection {
            Selection::Output(o) => o,
            _ => {
                let ni = usize::from(self.state.caps.num_inputs);
                (0..outputs)
                    .find(|o| !screens::channel_unused(&self.state, ni + o))
                    .unwrap_or(0)
            }
        };
        self.handle_event(session, ShellEvent::Select(Selection::Output(o)));
        self.shell.focus = crate::shell::Focus::Screen;
        let x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let ev = self.shell.detail.handle(x, &self.state);
        if ev == crate::shell::ScreenEvent::Unhandled {
            self.note("This device has no crossover tab");
        }
    }

    /// One of the interface's own verbs: the Console's File, Tools and AutoEQ
    /// menus.
    ///
    /// Each one either acts at once or raises the first dialog of its flow;
    /// the rest of each flow is in `finish_dialog`, so a whole flow reads in
    /// one place rather than being spread through the key handler.
    fn app_verb(&mut self, session: &mut Session, verb: &str, args: &[&str]) {
        match verb {
            "import" => self.ask_path(session, FileAction::ImportFilters, args),
            "export" => self.ask_path(session, FileAction::ExportFilters, args),
            "import-config" => self.ask_path(session, FileAction::ImportConfig, args),
            "export-config" => self.ask_path(session, FileAction::ExportConfig, args),
            // -- WP FIN015 -- no question about slot 1 for an unknown one
            "commit" => match self.known_active() {
                Some(slot) => self.dialog = Some((AppDialog::Commit, actions::commit_dialog(slot))),
                None => self.note(presets::ACTIVE_UNKNOWN),
            },
            "revert" => self.dialog = Some((AppDialog::Revert, actions::revert_dialog())),
            "factory-reset" => {
                self.dialog = Some((AppDialog::FactoryReset, actions::factory_reset_dialog()))
            }
            "bootloader" => self.dialog = Some((AppDialog::Bootloader, actions::firmware_dialog())),
            "save-master" => match actions::save_master_volume(session, &self.state) {
                Ok(m) | Err(m) => self.note(m),
            },
            "save-output-config" => match actions::save_output_config(session) {
                Ok(m) => {
                    self.state.limiter_saved();
                    self.note(m)
                }
                Err(m) => self.note(m),
            },
            // -- WP TMCOREF --
            "unlock" => self.ask_unlock(session),
            // -- WP UTM --
            "help" => self.shell.help = true,
            "device" => self.open_device_picker(),
            "subharm" => self.open_tool(Tool::Subharm),
            "autoeq" => match args.first().copied() {
                Some("update") => self.autoeq_update(),
                _ => self.open_tool(Tool::AutoEq),
            },
            "tube" => self.open_tool(Tool::Tube),
            // -- WP TM8 -- The way back from the status line's key, or from a
            // hub that gave up. Only USB has a device to switch to by serial.
            "reconnect" if self.offline_from_hub() => {
                if self.reconnect.is_none() {
                    // -- WP TMCORE -- through the embedder's opener
                    self.reconnect = self.target.clone().map(|t| self.reconnector(t));
                    self.echo(String::new());
                }
                self.sync_model();
            }
            "reconnect" if matches!(self.target, Some(Target::Link(_))) => {
                self.note(link_words::BOUND.text);
            }
            "reconnect" => self.switch_to = Some(self.state.caps.serial.clone()),
            other => self.note(format!("`{other}` is not one of this interface's verbs")),
        }
    }

    /// Raise a path dialog, or act at once when the path came with the verb:
    /// `:export tuning.txt` should not stop to ask.
    fn ask_path(&mut self, session: &mut Session, action: FileAction, args: &[&str]) {
        if let Some(path) = args.first().filter(|p| !p.is_empty()) {
            self.run_file_action(session, action, path);
            return;
        }
        let (title, body, default, button) = match action {
            FileAction::ImportFilters => (
                "Import Filters",
                "A REW, DSPi or DSPi for Windows filter file.",
                String::new(),
                "Import",
            ),
            FileAction::ExportFilters => (
                "Export Filters",
                "Where to write this device's filters.",
                actions::DEFAULT_FILTER_NAME.to_string(),
                "Export",
            ),
            FileAction::ImportConfig => (
                "Import Device Configuration",
                "A .dspipreset document.",
                String::new(),
                "Import",
            ),
            FileAction::ExportConfig => (
                "Export Device Configuration",
                "Where to write this device's whole configuration.",
                actions::DEFAULT_CONFIG_NAME.to_string(),
                "Export",
            ),
            FileAction::ImportAutoEqDatabase => (
                "Import AutoEQ Database",
                "Select an autoeq_database.json file.",
                String::new(),
                "Import",
            ),
        };
        self.dialog = Some((
            AppDialog::Path(action),
            actions::path_dialog(title, body, &default, button),
        ));
    }

    /// A path has been settled on: read it, write it, or move to the step that
    /// needs to ask something else.
    fn run_file_action(&mut self, session: &mut Session, action: FileAction, path: &str) {
        match action {
            FileAction::ExportFilters => match actions::export_filters(&self.state, path) {
                Ok(m) => self.note(m),
                Err(e) => self.note(e),
            },
            FileAction::ExportConfig => {
                let linked = self.shared.borrow().linked_pairs;
                match actions::export_config(session, path, &linked) {
                    Ok(m) => self.note(m),
                    Err(e) => self.note(e),
                }
            }
            FileAction::ImportFilters => {
                let text = match std::fs::read_to_string(actions::expand(path)) {
                    Ok(t) => t,
                    Err(e) => return self.note(format!("Failed to read file: {e}")),
                };
                match dspi_session::filterfile::parse(&text) {
                    Ok(file) => {
                        let (dialog, targets) = actions::channel_picker(&file, &self.state);
                        if targets.is_empty() {
                            return self.note("No valid filters found in file");
                        }
                        self.dialog = Some((
                            AppDialog::ImportChannels {
                                file: Box::new(file),
                                targets,
                            },
                            dialog,
                        ));
                    }
                    Err(_) => self.note("Failed to parse DSPi Console filter file"),
                }
            }
            FileAction::ImportConfig => match actions::read_config(path) {
                Ok(doc) => {
                    let dialog = actions::import_options_dialog(&doc, &self.state);
                    self.dialog = Some((AppDialog::ConfigOptions(Box::new(doc)), dialog));
                }
                Err(e) => self.note(e),
            },
            FileAction::ImportAutoEqDatabase => {
                match dspi_session::autoeq::import_database(&actions::expand(path)) {
                    Ok(n) => {
                        self.reload_autoeq();
                        self.note(format!("Database updated successfully.  Entries: {n}"));
                    }
                    Err(e) => self.note(format!("Failed to import database: {e}")),
                }
            }
        }
    }

    /// Throw away the cached database so the next look at it re-reads.
    fn reload_autoeq(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.autoeq = None;
        shared.favourites = dspi_session::autoeq::load_favourites();
    }

    fn open_device_picker(&mut self) {
        if self.target.as_ref().is_some_and(|t| !t.is_usb()) {
            self.note("One device connected");
            return;
        }
        let devices = actions::device_list();
        if devices.len() < 2 {
            self.note(if devices.is_empty() {
                "No devices found"
            } else {
                "One device connected"
            });
            return;
        }
        let serials: Vec<String> = devices.iter().map(|d| d.serial.clone()).collect();
        let items: Vec<String> = devices
            .iter()
            .map(|d| format!("{}  {}", d.short_name(), d.bus_id))
            .collect();
        let here = self.state.caps.serial.clone();
        let cursor = serials.iter().position(|s| *s == here).unwrap_or(0);
        self.dialog = Some((
            AppDialog::DevicePicker(serials),
            Dialog::list("Device", "", items, cursor),
        ));
    }

    /// The Console's Update Database menu, with its three methods.
    fn autoeq_update(&mut self) {
        let db = self.shared.borrow().autoeq.clone();
        let (count, date) = match &db {
            Some(d) => (
                d.entries.len(),
                if d.generated_at.is_empty() {
                    "Unknown".to_string()
                } else {
                    d.generated_at.clone()
                },
            ),
            None => (0, "Unknown".to_string()),
        };
        let mut methods = vec![AutoEqMethod::Rebuild, AutoEqMethod::Import];
        // Only offered once there is a user copy to throw away, as the
        // Console's menu does.
        if dspi_session::autoeq::has_user_database() {
            methods.push(AutoEqMethod::Reset);
        }
        let mut buttons: Vec<Button> = methods
            .iter()
            .map(|m| {
                Button::new(match m {
                    AutoEqMethod::Rebuild => "Rebuild from GitHub",
                    AutoEqMethod::Import => "Import File...",
                    AutoEqMethod::Reset => "Reset to Built-in",
                })
            })
            .collect();
        buttons.push(Button::new("Cancel"));
        self.dialog = Some((
            AppDialog::AutoEqUpdate(methods),
            Dialog::confirm(
                "Update AutoEQ Database",
                format!("Current database: {date}\nEntries: {count}\n\nChoose an update method:"),
                buttons,
            ),
        ));
    }

    fn select(&mut self, sel: Selection) {
        self.shell.model.selection = sel;
        self.shell.detail = self.screens.detail(&self.state, sel);
        if let Some(row) = self.shell.model.row_of_selection() {
            self.shell.sidebar_cursor = row;
        }
        self.sync_model();
    }

    /// Ask before losing unsaved changes, in the Console's words.
    fn unsaved_dialog(&self) -> Dialog {
        let diff = self.state.unsaved_diff();
        let summary = dspi_session::PresetSnapshot::summary(&diff, 6);
        Dialog::confirm(
            "Unsaved Changes",
            format!(
                "The current preset has unsaved changes:\n\n{summary}\n\nSave before continuing?"
            ),
            vec![
                Button::new("Save"),
                Button::destructive("Discard"),
                Button::new("Cancel"),
            ],
        )
    }

    // -- WP FIN015 --
    /// The active slot when it is one the device has; `None` when it is
    /// unknown (a failed read, or 10 or more). Nothing is saved, cleared,
    /// renamed or copied "to the active preset" then (REVF S1).
    fn known_active(&self) -> Option<u8> {
        dspi_session::preset_copy::known_slot(self.state.caps.active_preset)
    }

    fn save_active_preset(&mut self, session: &mut Session) -> bool {
        let Some(slot) = self.known_active() else {
            self.note("No active preset slot");
            return false;
        };
        if !self.save_slot(session, slot) {
            self.note("Save Failed");
            return false;
        }
        self.state.mark_saved();
        self.echo_set("preset.save", &[slot], Value::Trigger);
        self.sync_model();
        true
    }

    /// Load a preset slot, wait for the device to finish, and take what it
    /// loaded as the new baseline. False when it did not load.
    ///
    /// The load is deferred: the device answers at once and loads after its
    /// fade and DAC mute hold (vendor_commands.c:2698-2716,
    /// main.c:1182-1186, 2880-2928), so a re-read straight away would see the
    /// old state. The end is its `BULK_INVALIDATED` (flash_storage.c:4375).
    fn load_preset(&mut self, session: &mut Session, slot: u8) -> bool {
        self.flush_writes(session, true);
        // -- WP TMCORE -- a routed session loads through its route (the
        // console core: allowed while locked, waits for the load, holds the
        // slot's values to the limits profile)
        if session.is_routed() {
            if let Err(e) = session.write("preset.load", &[slot], Value::Trigger) {
                self.note(format!("Load Failed: {e}"));
                return false;
            }
            return self.loaded(session, slot);
        }
        let status = session.with_transport(|t| {
            t.control_in(
                dspi_proto::generated::opcodes::REQ_PRESET_LOAD,
                slot as u16,
                1,
            )
        });
        // The request answers only accepted or a bad slot.
        if !matches!(status.map(|b| b.first().copied()), Ok(Some(0))) {
            self.note("Load Failed");
            return false;
        }
        self.wait_for_bulk(Source::Preset);
        self.loaded(session, slot)
    }

    // -- WP TMCORE -- (the end of `load_preset`, shared with the routed load)
    /// After a load of `slot`: the image read again and the slot checked.
    fn loaded(&mut self, session: &mut Session, slot: u8) -> bool {
        if !self.refresh(session) {
            self.state.stale = true;
            return false;
        }
        // A slot that fails its CRC check still sends PRESET_LOADED and
        // BULK_INVALIDATED, but the device keeps its active slot and its live
        // state (flash_storage.c:4316-4327); that is the only sign.
        let active = read_active_slot(session);
        if let Some(a) = active.filter(|a| *a != slot) {
            self.state.caps.active_preset = Some(a);
            // PRESET_ERR_CRC: the Console's own words.
            self.note("Load Failed: Preset data is corrupted.");
            self.sync_model();
            return false;
        }
        self.state.caps.active_preset = Some(slot);
        self.state.mark_saved();
        self.echo_set("preset.load", &[slot], Value::Trigger);
        self.sync_model();
        true
    }

    fn strip_path(&self, i: usize) -> Option<&'static str> {
        match i {
            1 => Some("cf.on"),
            2 => Some("loud.on"),
            3 => Some("lev.on"),
            4 => Some("bass.on"),
            7 => Some("bypass"),
            _ => None,
        }
    }

    fn strip_tool(&self, i: usize) -> Option<Tool> {
        match i {
            0 => Some(Tool::Matrix),
            1 => Some(Tool::Crossfeed),
            2 => Some(Tool::Loudness),
            3 => Some(Tool::Leveller),
            4 => Some(Tool::Psybass),
            5 => Some(Tool::Stats),
            _ => None,
        }
    }

    /// Turn a shell event into device traffic or a state change.
    pub fn handle_event(&mut self, session: &mut Session, ev: ShellEvent) {
        // Anything but a screen's writes or the volume slider sends the held
        // writes first, so a toggle, a reset or a preset action lands after
        // the values the person moved before it.
        if !matches!(ev, ShellEvent::Command(_) | ShellEvent::VolumeChanged(_)) {
            self.flush_writes(session, true);
        }
        match ev {
            ShellEvent::Select(sel) => self.select(sel),
            ShellEvent::GraphPartner => {
                self.partner_shown = !self.partner_shown;
                self.sync_model();
            }
            ShellEvent::StripToggle(i) => {
                if let Some(path) = self.strip_path(i) {
                    let on = self.shell.model.strip[i].state.unwrap_or(false);
                    self.set(session, path, &[], Value::Bool(!on));
                } else {
                    // The Console's click on a plain tool button opens it.
                    self.handle_event(session, ShellEvent::StripOpen(i));
                }
            }
            ShellEvent::StripOpen(i) => {
                if i == 6 {
                    self.open_settings(session);
                } else if let Some(tool) = self.strip_tool(i) {
                    self.open_tool(tool);
                }
            }
            ShellEvent::OpenTool(tool) => self.open_tool(tool),
            ShellEvent::CloseTool => self.shell.close_tool(),
            ShellEvent::OpenSettings => self.open_settings(session),
            ShellEvent::CloseSettings => self.shell.close_settings(),
            ShellEvent::Preset(Some(delta)) => {
                let cur = self.state.caps.active_preset.unwrap_or(0) as i32;
                let next = (cur + delta).clamp(0, 9) as u8;
                if next as i32 != cur {
                    self.request_preset(session, next);
                }
            }
            ShellEvent::Preset(None) => {
                let active = self.known_active(); // -- WP FIN015 --
                let dirty = self.state.has_unsaved_changes();
                let (popup, choices) = PresetMenu::popup(&self.shared.borrow(), active, dirty);
                self.popup = Some((AppDialog::PresetList(choices), popup));
            }
            ShellEvent::Source(Some(delta)) => {
                if let Some((choices, idx)) = &self.shell.model.source {
                    let next = (*idx as i32 + delta).clamp(0, choices.len() as i32 - 1) as u8;
                    if next as usize != *idx {
                        self.set(session, "in.source", &[], Value::Choice(next));
                    }
                }
            }
            ShellEvent::Source(None) => {
                if let Some((choices, idx)) = &self.shell.model.source {
                    let popup = PopupList::new("Source", choices.clone(), *idx);
                    self.popup = Some((AppDialog::SourceList, popup));
                }
            }
            ShellEvent::VolumeChanged(db) => {
                let path = match self.shell.model.volume_mode {
                    VolumeMode::User => "vol.user",
                    VolumeMode::Master => "vol.master",
                };
                let line = format!("{path} {}", screens::number(db as f32));
                self.run_screen_commands(session, &line);
            }
            ShellEvent::VolumeReset => {
                let path = match self.shell.model.volume_mode {
                    VolumeMode::User => "vol.user",
                    VolumeMode::Master => "vol.master",
                };
                self.set(session, path, &[], Value::Float(0.0));
            }
            ShellEvent::VolumeMute => {
                let muted = self.state.user_volume().1;
                self.set(session, "vol.mute", &[], Value::Bool(!muted));
            }
            // -- WP TM18 --
            ShellEvent::VolumeModeToggle => {
                use crate::settings::config::VolumeChoice;
                let choice = match self.shell.model.volume_mode {
                    VolumeMode::User => VolumeChoice::Master,
                    VolumeMode::Master => VolumeChoice::User,
                };
                self.shell.model.volume_mode = volume_mode_of(choice);
                let saved = self.screens.update_config(&mut |c| c.volume.mode = choice);
                if let Err(e) = saved {
                    self.note(format!("Could not save settings: {e}"));
                }
                self.volume_choice = self.screens.config().volume.mode;
                self.sync_model();
            }
            ShellEvent::Command(c) => self.run_screen_commands(session, &c),
            ShellEvent::Session(req, owner) => {
                let reply = (req.run)(session);
                self.refresh(session);
                let follow = self.shell.deliver(owner, req.tag, reply, &self.state);
                for ev in follow {
                    self.handle_event(session, ev);
                }
            }
            ShellEvent::Status(s) => self.note(s),
            ShellEvent::Palette => self.prompt = Some(Prompt::new(true, &self.ctx)),
            ShellEvent::CommandLine => self.prompt = Some(Prompt::new(false, &self.ctx)),
            ShellEvent::SavePreset => {
                // Ctrl-S and `:commit` are the Console's one Commit Parameters
                // action, so they raise one dialog rather than two copies of
                // the same wording that could drift apart.
                // -- WP FIN015 -- no question about slot 1 for an unknown one
                match self.known_active() {
                    Some(slot) => {
                        self.dialog = Some((AppDialog::Commit, actions::commit_dialog(slot)))
                    }
                    None => self.note(presets::ACTIVE_UNKNOWN),
                }
            }
            ShellEvent::DevicePicker => self.open_device_picker(),
            ShellEvent::Undo => self.undo(session, false),
            ShellEvent::Redo => self.undo(session, true),
            ShellEvent::Quit => {
                if self.state.has_unsaved_changes() {
                    self.dialog = Some((
                        AppDialog::Unsaved {
                            then: PendingAction::Quit,
                        },
                        self.unsaved_dialog(),
                    ));
                } else {
                    self.should_quit = true;
                }
            }
            ShellEvent::ClearClips => {
                self.state.clear_clip_latch();
                let now = Instant::now();
                self.meters.clear_clips(now);
                let sent = session.write("meters.clear", &[], Value::Trigger);
                // A refused clear (a viewer on a hub) brings the clips back.
                self.meters.clear_result(now, sent.is_ok());
                self.state.clip_latched = self.meters.clip_hold();
                self.sync_model();
            }
            ShellEvent::BypassToggle => {
                let on = self.state.global().bypass;
                self.set(session, "bypass", &[], Value::Bool(!on));
            }
            ShellEvent::GraphCursor(hz) => {
                self.shell.model.cursor_hz = hz;
                if let Some(hz) = hz {
                    let m = &self.shell.model;
                    let g = crate::graph::Graph::new(&m.curves, &m.graph, &self.shell.theme);
                    // Graphing's two readout switches; with both off the
                    // cursor line alone is drawn.
                    if let Some(text) = g.readout_text(hz) {
                        self.shell.model.status = Some(text);
                        self.status_until = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
            }
            // -- WP TM5 --
            ShellEvent::SpectrumOverlay => self.toggle_spectrum_overlay(session),
            ShellEvent::TakeControl => self.take_analyser_control(),
            // -- WP TM6 --
            ShellEvent::SpectrumBars => self.toggle_spectrum_bars(session),
            ShellEvent::GraphZoom(delta) => self.shell.model.graph.zoom(delta),
            ShellEvent::GraphPhase => {
                self.shell.model.graph.show_phase = !self.shell.model.graph.show_phase;
                self.sync_model();
            }
            ShellEvent::GraphHeight => {
                self.shell.model.graph_height = self.shell.model.graph_height.next();
            }
            ShellEvent::GraphPopout => {
                self.shell.graph_popout = !self.shell.graph_popout;
                // The pop-out stays on the channel it opened on unless
                // Graphing says it follows the selection (the Console's
                // setting).
                self.popout_pinned = if self.shell.graph_popout
                    && !self.screens.config().graphing.popout_follows_selection
                {
                    self.shell.model.row_of_selection()
                } else {
                    None
                };
                self.sync_model();
            }
            ShellEvent::Rename(row) => {
                let name = self.state.channel_name(row);
                self.dialog = Some((
                    AppDialog::Rename { channel: row },
                    Dialog::text("Rename", "", name, "Name"),
                ));
            }
            ShellEvent::CopyParams(row) => {
                let clip = clipboard::copy(&self.state, row);
                self.note(clipboard::copied_message(&clip));
                self.shared.borrow_mut().clipboard = Some(clip);
            }
            ShellEvent::PasteParams(row) => {
                let clip = self.shared.borrow().clipboard.clone();
                let Some(clip) = clip else {
                    self.note("Nothing to paste");
                    return;
                };
                // A linked pair mirrors every other edit, so a paste has to
                // mirror too or the two halves drift apart.
                let mirror = self
                    .shared
                    .borrow()
                    .linked_partner(row, self.state.caps.num_inputs as usize);
                let cmds = clipboard::paste_commands(&clip, &self.state, row, mirror);
                self.run_commands(session, &cmds.join("\n"));
                let name = screens::channel_label(&self.state, row);
                self.note(format!("Pasted {} onto {name}", clip.source));
            }
            ShellEvent::Identify(row) => self.identify(session, row),
        }
    }

    /// Play the channel-ID tone on one output, which is the Console's
    /// Identify: a counted blip melody so the listener can tell which speaker
    /// is which. It is only offered while the generator exists.
    fn identify(&mut self, session: &mut Session, row: usize) {
        let ni = self.state.caps.num_inputs as usize;
        let output = row.saturating_sub(ni);
        let name = screens::channel_label(&self.state, row);
        match screens::identify_command(&self.state, output) {
            Some(cmd) => {
                self.run_commands(session, &cmd);
                self.note(format!("Identifying {name}"));
            }
            None => self.note("Firmware has no signal generator"),
        }
    }

    fn request_preset(&mut self, session: &mut Session, slot: u8) {
        if self.state.has_unsaved_changes() {
            self.dialog = Some((
                AppDialog::Unsaved {
                    then: PendingAction::LoadPreset(slot),
                },
                self.unsaved_dialog(),
            ));
        } else {
            self.load_preset(session, slot);
        }
    }

    fn open_tool(&mut self, tool: Tool) {
        let screen = self.screens.tool(&self.state, tool);
        self.shell.open_tool(tool, screen);
    }

    /// Show a screen by name, as `uconsole screenshot` and the gallery ask:
    /// `overview`, `input` (the first input), `output` (the first output),
    /// a tool's lowercase title word (`matrix`, `crossfeed`, `loudness`,
    /// `leveller`, `psybass`, `upmixer`, `signals`, `stats`, `monitor`,
    /// `autoeq`, `subharm`, `tube`, `spectrum`), or `settings`. Returns false
    /// for a name it does not know.
    pub fn show(&mut self, session: &mut Session, name: &str) -> bool {
        match name.to_ascii_lowercase().as_str() {
            "overview" => self.select(Selection::Overview),
            "input" => self.select(Selection::Input(0)),
            "output" => self.select(Selection::Output(0)),
            "matrix" => self.open_tool(Tool::Matrix),
            "crossfeed" => self.open_tool(Tool::Crossfeed),
            "loudness" => self.open_tool(Tool::Loudness),
            "leveller" => self.open_tool(Tool::Leveller),
            "psybass" | "bass" => self.open_tool(Tool::Psybass),
            "upmixer" => self.open_tool(Tool::Upmixer),
            "signals" => self.open_tool(Tool::Signals),
            "stats" => self.open_tool(Tool::Stats),
            "monitor" => self.open_tool(Tool::Monitor),
            "autoeq" => self.open_tool(Tool::AutoEq),
            "subharm" => self.open_tool(Tool::Subharm),
            "tube" => self.open_tool(Tool::Tube),
            "spectrum" => {
                if !self.shared.borrow().spectrum.engine.supported() {
                    self.connect_analyser(session);
                }
                self.open_tool(Tool::Spectrum)
            }
            "settings" => self.open_settings(session),
            _ => return false,
        }
        true
    }

    fn open_settings(&mut self, session: &mut Session) {
        self.screens.refresh_settings(session);
        self.refresh_cs_aux(session);
        let screen = self.screens.settings(&self.state);
        self.shell.open_settings(screen);
    }

    // -- WP TMCOREF -- (review tmcore F1)
    /// Ask for the installer's PIN (never drawn), when the session's route
    /// has a lock to open.
    fn ask_unlock(&mut self, session: &Session) {
        if !session.is_routed() {
            self.note("Nothing here is locked by an installer.");
            return;
        }
        self.dialog = Some((
            AppDialog::Unlock,
            Dialog::secret(
                "Unlock",
                "The installer's PIN or recovery code. It unlocks this DSPi here until the \
                 connection ends.",
                "PIN",
            ),
        ));
    }

    // -- WP TMCOREF --
    /// Say why a write failed, with the route's action line; a write the
    /// installer lock refused offers the unlock prompt.
    fn note_write_error(&mut self, session: &Session, e: &dspi_session::WriteError) {
        match e {
            dspi_session::WriteError::Routed { action, kind, .. } => {
                if action.is_empty() {
                    self.note(e.to_string());
                } else {
                    self.note(format!("{e} {action}"));
                }
                if *kind == dspi_session::RouteFailure::Locked && self.dialog.is_none() {
                    self.ask_unlock(session);
                }
            }
            _ => self.note(e.to_string()),
        }
    }

    fn finish_dialog(&mut self, session: &mut Session, kind: AppDialog, outcome: DialogOutcome) {
        self.flush_writes(session, true);
        match (kind, outcome) {
            (AppDialog::Unsaved { then }, DialogOutcome::Button(0)) => {
                if self.save_active_preset(session) {
                    self.run_pending(session, then);
                }
            }
            (
                AppDialog::Unsaved {
                    then: PendingAction::CopyTo(dest),
                },
                DialogOutcome::Button(1),
            ) => {
                // -- WP FIN015 -- no load of slot 0 for an unknown one
                match self.known_active() {
                    Some(source) => {
                        if self.load_preset(session, source) {
                            self.copy_preset_to(session, dest);
                        }
                    }
                    None => self.note(presets::ACTIVE_UNKNOWN),
                }
            }
            (AppDialog::Unsaved { then }, DialogOutcome::Button(1)) => {
                self.run_pending(session, then)
            }
            // -- WP TMCOREF --
            (AppDialog::Unlock, DialogOutcome::Text(pin)) => match session.unlock(&pin) {
                Some(Ok(m)) => self.note(m),
                Some(Err(m)) => self.note(m),
                None => self.note("Nothing here is locked by an installer."),
            },
            (AppDialog::Rename { channel }, DialogOutcome::Text(name)) => {
                self.set(session, "ch.name", &[channel as u8], Value::Text(name));
                // The detail region names the channel in its title, so it has
                // to be rebuilt for the new name to show.
                let sel = self.shell.model.selection;
                self.shell.detail = self.screens.detail(&self.state, sel);
            }
            (AppDialog::PresetList(choices), DialogOutcome::Picked(i)) => {
                self.preset_action(session, choices.get(i).copied().flatten())
            }
            (AppDialog::PresetCopyTo(slots), DialogOutcome::Picked(i)) => {
                if let Some(dest) = slots.get(i).copied() {
                    self.copy_preset_to(session, dest);
                }
            }
            (AppDialog::PresetRename(slot), DialogOutcome::Text(name)) => {
                self.set(session, "preset.name", &[slot], Value::Text(name));
                self.refresh_presets(session);
            }
            (AppDialog::PresetClear(slot), DialogOutcome::Button(0)) => {
                self.clear_preset(session, slot);
                self.refresh_presets(session);
            }
            (
                AppDialog::Hazard {
                    path,
                    indices,
                    value,
                },
                DialogOutcome::Button(0),
            ) => {
                // -- WP TM19 --
                if self.write_set(session, &path, &indices, value) {
                    self.tell_written(session, &[path]);
                }
            }
            // -- WP TM19 --
            (AppDialog::HazardBlock { lines, note }, DialogOutcome::Button(0)) => {
                self.run_block(session, lines, note);
            }
            (AppDialog::HazardBlock { .. }, _) => self.note("Cancelled; nothing was written"),
            (AppDialog::SourceList, DialogOutcome::Picked(i)) => {
                self.set(session, "in.source", &[], Value::Choice(i as u8));
            }
            (AppDialog::Core1 { index }, DialogOutcome::Button(0)) => {
                match session.enable_output_confirmed(index) {
                    Ok(dspi_session::EnableOutcome::Done) => {
                        self.echo_set("out.enable", &[index], Value::Bool(true));
                        self.refresh(session);
                    }
                    Ok(_) => self.note("The device kept the output as it was"),
                    Err(e) => self.note(e.to_string()),
                }
            }
            (AppDialog::PresetClearAll, DialogOutcome::Button(0)) => {
                for slot in 0..presets::SLOTS as u8 {
                    self.clear_preset(session, slot);
                }
                self.refresh_presets(session);
            }

            // ---------------------------------------------------- file flows
            (AppDialog::Path(action), DialogOutcome::Text(path)) => {
                if !path.trim().is_empty() {
                    self.run_file_action(session, action, path.trim());
                }
            }
            (AppDialog::ImportChannels { file, targets }, DialogOutcome::Checked(checked, 0)) => {
                let targets: Vec<actions::ImportTarget> = targets
                    .into_iter()
                    .zip(checked)
                    .map(|(mut t, on)| {
                        t.checked = on;
                        t
                    })
                    .collect();
                let count = targets.iter().filter(|t| t.checked).count();
                let (commands, notes) = actions::import_commands(&file, &self.state, &targets);
                self.run_commands(session, &commands.join("\n"));
                let report = actions::import_report(&file, &self.state, count, &notes);
                self.note(report);
            }
            (AppDialog::ConfigOptions(doc), DialogOutcome::Checked(checked, 0)) => {
                let options = dspi_session::preset_file::ApplyOptions {
                    audio_processing: true,
                    volume_levels: checked.first().copied().unwrap_or(false),
                    hardware_io: checked.get(1).copied().unwrap_or(false),
                };
                // The Console shows a progress sheet here. Applying is
                // synchronous over the same control endpoint everything else
                // uses, so what a bar would animate is a few hundred
                // milliseconds of transfers; the report is the part that
                // matters and it comes up when they are done.
                if options.hardware_io {
                    // Imported limiters are live but unsaved output
                    // configuration (PresetDocumentTransfer.swift:438).
                    self.state.begin_limiter_edit();
                }
                // -- WP TMCORE -- one undo step for the whole import
                session.begin_group("Import Device Configuration");
                // -- WP TMCOREF2 -- held to the route's limits profile first
                // (the console core's), as `uconsole import` holds it: a value
                // outside it is written at the limit and the report says so
                // ("Held to the limits profile: ...")
                let profile = session.limits_profile();
                let report = dspi_session::preset_file::apply_limited(
                    session,
                    &doc,
                    options,
                    profile.as_ref(),
                );
                session.end_group();
                // The pair links are the app's, so the device apply leaves
                // them here (PresetDocumentTransfer.swift:367-375).
                let mut shared = self.shared.borrow_mut();
                for pair in 0..shared.linked_pairs.len() {
                    let on = doc.global.input_pair_linked.get(pair).copied();
                    shared.set_linked(pair, on.unwrap_or(false));
                }
                drop(shared);
                self.refresh(session);
                self.dialog = Some((
                    AppDialog::Report,
                    Dialog::report("Import Device Configuration", report.lines(false)),
                ));
            }

            // --------------------------------------------------------- tools
            (AppDialog::Commit, DialogOutcome::Button(0)) => {
                if self.save_active_preset(session) {
                    self.note("Preset saved successfully");
                }
            }
            (AppDialog::Revert, DialogOutcome::Button(0)) => {
                // The old synchronous REQ_LOAD_PARAMS crashed the device on
                // S/PDIF input and was reassigned; a host reverts by re-loading
                // the active slot, which is deferred and stream-safe
                // (config.h:201-204).
                match self.state.caps.active_preset {
                    Some(slot) => {
                        if self.load_preset(session, slot) {
                            self.note("Parameters reverted successfully");
                        }
                    }
                    None => self
                        .note("No saved parameters found.  The device is using factory defaults."),
                }
            }
            (AppDialog::FactoryReset, DialogOutcome::Button(0)) => self.factory_reset(session),
            (AppDialog::Bootloader, DialogOutcome::Button(0)) => {
                // The device answers, waits 100 ms and resets; there is nothing
                // to acknowledge and every later transfer will fail, which is
                // expected rather than an error (survey 6.2).
                let _ = session.write("dev.bootloader", &[], Value::Trigger);
                let watch = actions::BootloaderWatch::new();
                self.dialog = Some((
                    AppDialog::BootWait,
                    Dialog::progress("Reboot into Bootloader", watch.status()),
                ));
                self.boot = Some(watch);
            }
            (AppDialog::BootWait, _) => self.boot = None,
            (AppDialog::DevicePicker(serials), DialogOutcome::Picked(i)) => {
                if let Some(serial) = serials.get(i).cloned() {
                    self.switch_device(serial);
                }
            }

            // -------------------------------------------------------- autoeq
            (AppDialog::AutoEqUpdate(methods), DialogOutcome::Button(i)) => {
                match methods.get(i) {
                    Some(AutoEqMethod::Rebuild) => {
                        self.dialog = Some((
                            AppDialog::AutoEqRebuild,
                            Dialog::confirm(
                                "Rebuild AutoEQ Database",
                                dspi_session::autoeq::rebuild_warning(),
                                vec![Button::new("Rebuild"), Button::new("Cancel")],
                            )
                            .default_button(1),
                        ))
                    }
                    Some(AutoEqMethod::Import) => {
                        self.ask_path(session, FileAction::ImportAutoEqDatabase, &[])
                    }
                    Some(AutoEqMethod::Reset) => match dspi_session::autoeq::reset_to_builtin() {
                        Ok(n) => {
                            self.reload_autoeq();
                            self.note(format!("Reset to built-in database.  Entries: {n}"));
                        }
                        Err(e) => self.note(format!("Failed to reset: {e}")),
                    },
                    // Cancel is the button past the last method.
                    None => {}
                }
            }
            (AppDialog::AutoEqRebuild, DialogOutcome::Button(0)) => {
                // -- WP TM12 --
                let handle = dspi_session::autoeq::rebuild_from_github((self.autoeq_fetch)());
                self.dialog = Some((
                    AppDialog::AutoEqRebuildProgress,
                    Dialog::progress("Updating Database", "Connecting to GitHub..."),
                ));
                self.rebuild = Some(handle);
            }
            (AppDialog::AutoEqRebuildProgress, _) => {
                // Escape on the progress dialog stops the download rather than
                // leaving a thread hammering GitHub with nobody watching.
                self.rebuild = None;
                self.note("Rebuild cancelled");
            }
            _ => {}
        }
    }

    /// Switch to another device, asking first if this one has unsaved changes.
    fn switch_device(&mut self, serial: String) {
        if serial == self.state.caps.serial {
            return;
        }
        if self.state.has_unsaved_changes() {
            self.dialog = Some((
                AppDialog::Unsaved {
                    then: PendingAction::SwitchDevice(serial),
                },
                self.unsaved_dialog(),
            ));
        } else {
            self.switch_to = Some(serial);
        }
    }

    /// Open another device in place of this one.
    ///
    /// Everything that describes the old device goes: the capabilities, the
    /// bulk shadow, the meters, the preset names and the diagnostics. Keeping
    /// any of it would show one device's numbers under another one's name.
    pub fn adopt(&mut self, session: &mut Session, state: DeviceState) {
        let caps = &state.caps;
        self.ctx = Context {
            channel_slugs: caps.channels.iter().map(|c| c.slug.clone()).collect(),
            num_inputs: caps.num_inputs,
            num_outputs: caps.num_outputs,
            max_bands: caps.max_bands,
        };
        self.meters.reset();
        self.popout_pinned = None;
        // A held write was for the old device.
        self.gate = WriteGate::default();
        self.state = state;
        // The Console discards Settings drafts and device facts on a different
        // serial; the pages re-read on their next open.
        self.screens.refresh_settings(session);
        {
            let mut shared = self.shared.borrow_mut();
            shared.stats = actions::Stats::default();
            shared.preset_names.clear();
            shared.occupied = 0;
            shared.default_slot = None;
        }
        self.shell.model.connected = true;
        self.select(Selection::Overview);
        self.refresh_presets(session);
        self.note(format!("Switched to {}", self.state.caps.serial));
    }

    /// One item of the Preset row's menu.
    fn preset_action(&mut self, session: &mut Session, choice: Option<PresetChoice>) {
        // -- WP FIN015 -- the menu offers these only on a known active slot;
        // slot 0 never stands in for an unknown one (REVF S1)
        let on_active = !matches!(
            choice,
            None | Some(PresetChoice::Slot(_) | PresetChoice::ClearAll)
        );
        let active = match self.known_active() {
            Some(a) => a,
            None if on_active => {
                self.note(presets::ACTIVE_UNKNOWN);
                return;
            }
            // Unused by the slot rows and Clear All.
            None => 0,
        };
        match choice {
            Some(PresetChoice::Slot(slot)) => self.request_preset(session, slot),
            Some(PresetChoice::Save) => {
                // The Console names an unnamed slot before saving it, so a
                // saved preset never shows as Empty.
                if PresetMenu::dropdown_label(&self.shared.borrow(), active) == "Empty" {
                    self.set(
                        session,
                        "preset.name",
                        &[active],
                        Value::Text(format!("Preset {}", active + 1)),
                    );
                }
                if self.save_active_preset(session) {
                    self.refresh_presets(session);
                }
            }
            Some(PresetChoice::Rename) => {
                let d = PresetMenu::rename_dialog(&self.shared.borrow(), active);
                self.dialog = Some((AppDialog::PresetRename(active), d));
            }
            Some(PresetChoice::SetDefault) => {
                // `WirePresetStartup`: mode 0 is "a specified slot".
                self.set(
                    session,
                    "preset.startup",
                    &[],
                    Value::Bytes(vec![0, active]),
                );
                self.shared.borrow_mut().default_slot = Some(active);
            }
            Some(PresetChoice::CopyTo) => {
                let (popup, slots) = PresetMenu::copy_to_popup(&self.shared.borrow(), active);
                self.popup = Some((AppDialog::PresetCopyTo(slots), popup));
            }
            Some(PresetChoice::Clear) => {
                let label = PresetMenu::slot_label(&self.shared.borrow(), active);
                self.dialog = Some((
                    AppDialog::PresetClear(active),
                    PresetMenu::clear_dialog(&label),
                ));
            }
            Some(PresetChoice::ClearAll) => {
                self.dialog = Some((AppDialog::PresetClearAll, PresetMenu::clear_all_dialog()));
            }
            None => {}
        }
    }

    /// Copy the live parameters into another slot.
    ///
    /// Saving to the destination makes the firmware treat it as the last
    /// active slot, so the source is re-saved afterwards to put that back.
    /// Data-wise the second write is a no-op.
    ///
    /// Only from the known active slot: the second save writes over the
    /// source, so the copy refuses, by the core's rule and words
    /// (`dspi_session::preset_copy`), when the active slot is unknown or is
    /// no longer the one shown, before anything is written (WP FIN015, REVF S1).
    fn copy_preset_to(&mut self, session: &mut Session, dest: u8) {
        let Some(source) = self.known_active() else {
            self.note(presets::ACTIVE_UNKNOWN);
            return;
        };
        if dest == source {
            return;
        }
        // The second save below writes the live state over the source slot,
        // so the live state must be the source's saved state first: ask, as
        // the Console does, and on Discard reload the source before copying.
        if self.state.has_unsaved_changes() {
            self.dialog = Some((
                AppDialog::Unsaved {
                    then: PendingAction::CopyTo(dest),
                },
                self.unsaved_dialog(),
            ));
            return;
        }
        // -- WP FIN015 -- the device's own word, right before the first save
        let now = read_active_slot(session);
        if let Some(why) = dspi_session::preset_copy::copy_refusal(source, now) {
            if now.is_some() {
                self.state.caps.active_preset = now;
                self.sync_model();
            }
            self.note(format!("Copy Failed: {why}"));
            return;
        }
        // Each save is waited for: the device keeps one pending save, so the
        // second would otherwise replace the first before it was written.
        if !self.save_slot(session, dest) || !self.save_slot(session, source) {
            self.note("Save Failed");
            self.refresh_presets(session);
            return;
        }
        self.refresh_presets(session);
        self.echo_set("preset.save", &[dest], Value::Trigger);
    }

    /// Reset the device to factory defaults, wait for it to finish, and take
    /// the result as the new baseline. The reset is deferred behind the same
    /// fade as a load (vendor_commands.c:2396-2403, main.c:3130-3132) and
    /// ends with a `BULK_INVALIDATED` from the factory source
    /// (flash_storage.c:4947-4953).
    fn factory_reset(&mut self, session: &mut Session) {
        self.flush_writes(session, true);
        if session.write("dev.reset", &[], Value::Trigger).is_err() {
            self.note("Failed to reset parameters");
            return;
        }
        self.wait_for_bulk(Source::Factory);
        if self.refresh(session) {
            self.state.mark_saved();
            self.note("Factory reset complete");
        } else {
            self.state.stale = true;
        }
    }

    fn clear_preset(&mut self, session: &mut Session, slot: u8) {
        if session
            .write("preset.delete", &[slot], Value::Trigger)
            .is_err()
        {
            self.note("Clear Failed");
        }
    }

    /// Read the slot names back, which is where the Preset menu's labels come
    /// from. Called on connect and after anything that changes them.
    pub fn refresh_presets(&mut self, session: &mut Session) {
        let names: Vec<String> = (0..presets::SLOTS as u8)
            .map(|slot| match session.read("preset.name", &[slot]) {
                Ok(Value::Text(t)) => t.trim().to_string(),
                _ => String::new(),
            })
            .collect();
        // The directory says which slots hold anything and which one the
        // device loads at power on (config.h REQ_PRESET_GET_DIR, 7 bytes).
        let dir = session
            .with_transport(|t| {
                t.control_in(
                    dspi_proto::generated::opcodes::REQ_PRESET_GET_DIR,
                    0,
                    dspi_proto::packets::PresetDirectory::SIZE as u16,
                )
            })
            .ok()
            .and_then(|d| dspi_proto::packets::PresetDirectory::decode(&d).ok());
        {
            let mut shared = self.shared.borrow_mut();
            shared.preset_names = names;
            if let Some(dir) = &dir {
                shared.occupied = dir.occupied;
                shared.default_slot = (dir.startup_mode == 0).then_some(dir.default_slot);
            }
        }
        // The same packet says whether a limiter or master-volume edit is a
        // preset change.
        if let Some(dir) = dir {
            self.state.output_config_mode = dir.output_config_mode;
            self.state.master_volume_mode = dir.master_volume_mode;
        }
        self.sync_model();
    }

    fn run_pending(&mut self, session: &mut Session, then: PendingAction) {
        match then {
            PendingAction::Quit => self.should_quit = true,
            PendingAction::LoadPreset(slot) => {
                self.load_preset(session, slot);
            }
            PendingAction::SwitchDevice(serial) => self.switch_to = Some(serial),
            PendingAction::CopyTo(dest) => self.copy_preset_to(session, dest),
        }
    }

    /// The notification reader for the current session, started or
    /// restarted; the old one is dropped first so its interface is released.
    pub fn arm_notifications(&mut self, session: &mut Session) {
        self.notes = None;
        (self.link_kind, self.link_session) = session
            .with_transport(|t| Ok((t.caps().kind, t.session_id())))
            .unwrap_or((dspi_transport::TransportCaps::USB.kind, None));
        self.notes = session
            .with_transport(|t| Ok(t.notifications()))
            .ok()
            .flatten()
            .and_then(|source| Notifications::start(source).ok());
    }

    /// Where this device came from and what its extension channel offers:
    /// the title bar's words, the echo filter's mode, and what to reopen when
    /// it is lost. A `target` still open to any device is held to this one.
    pub fn set_connection(&mut self, target: &Target, ext: &ExtState) {
        let target = target.pinned(&self.state.caps.serial);
        self.shell.model.via = target.via();
        // -- WP SECTM -- a hub session without TLS: the badge, and once why
        let unsecured = matches!(&target, Target::Link(l) if l.unsecured()); // -- WP SECFIX -- not on loopback
        if unsecured && !self.shell.model.link_unsecured {
            let words = connect::unsecured_words();
            self.note(format!("{} {}", words.info, words.action));
        }
        self.shell.model.link_unsecured = unsecured;
        self.shell.model.ext = connect::ext_label(ext);
        self.echo = EchoFilter::default();
        self.echo.set_bulk_source(ext.bulk_source());
        self.reconnect = Some(self.reconnector(target.clone())); // -- WP TMCORE --
        self.target = Some(target);
        self.offline = false;
    }

    // -- WP PGTM --
    /// The open device's protocol for Settings > About, as the embedder read
    /// it at connect (`Connected::protocol`); `None` clears it.
    pub fn set_protocol(&mut self, protocol: Option<crate::settings::about::DeviceProtocolView>) {
        self.screens.set_protocol(protocol);
    }

    // -- WP P1TM --
    /// The session edits a document offline (`uconsole open FILE`): the
    /// title says "File: <name>" and the link reads `file`. No reconnect:
    /// there is no device to come back.
    pub fn set_document(&mut self, name: &str) {
        self.shell.model.document = Some(connect::printable(name));
        self.shell.model.via = "file".into();
        self.shell.model.link_unsecured = false; // -- WP SECTM --
    }

    /// Bring a lost device back, once one has been lost. Never blocks: the
    /// open runs on the reconnector's thread.
    pub fn advance_reconnect(&mut self, session: &mut Session) {
        if self.state.connected || self.reconnect.is_none() {
            return;
        }
        if !self.offline {
            self.go_offline(session);
        }
        // A bootloader handoff is the device leaving on purpose, and its
        // dialog is on screen: let that finish first.
        if self.boot.is_some() {
            return;
        }
        let Some(r) = self.reconnect.as_mut() else {
            return;
        };
        let polled = r.poll(Instant::now());
        // Only when an attempt has ended: a warning shown now would be replaced
        // by the outcome's note on a later poll.
        let warned = if matches!(polled, Reconnect::Waiting) {
            String::new()
        } else {
            r.take_warnings().iter().map(|w| format!("; {w}")).collect()
        };
        match polled {
            Reconnect::Waiting => {}
            Reconnect::GaveUp(e) => {
                self.reconnect = None;
                // On the echo line too, which stays: a status fades, and this
                // is why the title bar goes on saying "Not connected".
                let why = format!("Not reconnecting: {e}{warned}");
                self.echo(why.clone());
                self.note(why);
            }
            Reconnect::Connected(c) => {
                let c = *c;
                *session = c.session;
                self.adopt(session, c.state);
                self.set_protocol(c.protocol); // -- WP PGTM --
                if let Some(target) = self.target.clone() {
                    // -- WP TM9 -- A hub may have come back at another address.
                    let target = c.link.clone().map_or(target, Target::Link);
                    self.set_connection(&target, &c.ext);
                }
                self.arm_notifications(session);
                self.connect_analyser(session);
                self.note(format!(
                    "Reconnected to {}; undo history cleared{warned}",
                    self.state.caps.serial
                ));
            }
        }
    }

    /// Let go of a lost device's transport: its USB interface has to be free
    /// when it comes back, and a dead hub session has no use. Until then the
    /// polls fail at once, with no I/O.
    fn go_offline(&mut self, session: &mut Session) {
        self.offline = true;
        self.notes = None;
        // -- WP TM8 --
        // A held write fails with the link: it is never kept for the way back.
        let unsent = self.gate.held.len();
        self.gate = WriteGate::default();
        // Meters that stood still would read as live.
        self.state.meters.peaks.fill(0.0);
        self.state.meters.cpu0 = 0;
        self.state.meters.cpu1 = 0;
        self.meters.reset();
        let serial = self.state.caps.serial.clone();
        let offline = Box::new(connect::Offline::new(&serial));
        if let Some(s) = Session::new(offline, self.state.caps.clone()) {
            *session = s;
        }
        // -- WP TM8 --
        let unsent = match unsent {
            0 => String::new(),
            n => format!("; {n} unsent change(s) dropped"),
        };
        self.note(format!(
            "Lost {serial}; waiting for it to come back{unsent}"
        ));
    }

    /// Every attached device, for the picker and the title bar. Only USB can
    /// be enumerated for others: over any other link it is just this one.
    pub fn refresh_devices(&mut self) {
        if let Some(t) = self.target.as_ref().filter(|t| !t.is_usb()) {
            self.shell.model.devices = vec![format!("DSPi {}", t.via())];
            self.devices_checked = Some(Instant::now());
            return;
        }
        self.shell.model.devices = dspi_transport::list_devices()
            .map(|ds| {
                ds.iter()
                    .map(|d| format!("DSPi {}", d.short_name()))
                    .collect()
            })
            .unwrap_or_default();
        self.devices_checked = Some(Instant::now());
    }

    // -- WP TM8 --
    /// No connection, and the target is a hub.
    fn offline_from_hub(&self) -> bool {
        !self.state.connected && matches!(self.target, Some(Target::Link(_)))
    }

    /// A hub is lost and being retried: the state the status line, its key and
    /// the held mirror belong to. USB and MIDI keep the Console's no-device
    /// view: there the device has gone, not the road to it.
    fn link_reconnecting(&self) -> bool {
        self.offline_from_hub() && self.reconnect.is_some() && self.boot.is_none()
    }

    /// The status line's key. Drops the reconnector, so nothing more is dialled;
    /// a dial in progress closes itself when it finds nobody waiting.
    fn abort_reconnect(&mut self) {
        self.reconnect = None;
        self.echo(format!(
            "{}; :reconnect to try again",
            link_words::CLOSED.text
        ));
        self.sync_model();
    }

    /// One key, from the top of the overlay stack down.
    pub fn handle_key(&mut self, session: &mut Session, key: KeyEvent) {
        // -- WP TM8 -- Above every overlay: the way out is always there.
        if self.link_reconnecting() && crate::shell::is_disconnect_key(key) {
            self.abort_reconnect();
            return;
        }
        // -- WP UTM -- (UXP A13) Enter in the palette does what the find is
        if let Some(p) = &self.prompt
            && p.palette
            && key.code == KeyCode::Enter
        {
            match p.chosen(&self.ctx) {
                Some(action) => {
                    self.prompt = None;
                    self.palette_action(session, action);
                    return;
                }
                // Nothing found and not a command: the no-match line stays.
                None if p.hits.is_empty() && dspi_cmd::tokenize(&p.input).len() < 2 => {
                    if p.input.trim().is_empty() {
                        self.prompt = None;
                    }
                    return;
                }
                None => {}
            }
        }
        if let Some(p) = &mut self.prompt {
            if let Some(line) = p.handle(key, &self.ctx) {
                self.prompt = None;
                if !line.trim().is_empty() {
                    self.flush_writes(session, true);
                    self.run_command(session, &line);
                }
            }
            return;
        }
        if let Some((kind, d)) = &mut self.dialog {
            // A path field completes on Tab the way a shell does. The dialog
            // widget cannot do this itself: only the flow knows the field is a
            // path rather than a name.
            if key.code == KeyCode::Tab
                && matches!(kind, AppDialog::Path(_))
                && let crate::widgets::DialogKind::Text { value, .. } = &mut d.kind
                && let Some(completed) = actions::complete_path(value)
            {
                *value = completed;
                return;
            }
            if let Some(outcome) = d.handle(key) {
                let kind = kind.clone();
                self.dialog = None;
                self.finish_dialog(session, kind, outcome);
            }
            return;
        }
        if let Some((kind, p)) = &mut self.popup {
            match p.handle(key) {
                Some(crate::widgets::Action::Selected(i)) => {
                    let kind = kind.clone();
                    self.popup = None;
                    self.finish_dialog(session, kind, DialogOutcome::Picked(i));
                }
                Some(crate::widgets::Action::Closed) => self.popup = None,
                _ => {}
            }
            return;
        }
        let events = self.shell.handle(key, &self.state);
        for ev in events {
            self.handle_event(session, ev);
        }
        self.adopt_graphing();
        self.adopt_volume_choice();
    }

    // -- WP TM18 --
    /// Follow the volume mode when Settings has saved a new one.
    fn adopt_volume_choice(&mut self) {
        let choice = self.screens.config().volume.mode;
        if choice != self.volume_choice {
            self.volume_choice = choice;
            self.shell.model.volume_mode = volume_mode_of(choice);
            self.sync_model();
        }
    }

    /// Rebuild the graph's settings when Settings > Graphing has saved a
    /// change. The zoom and the phase toggle are the graph's own between
    /// saves; a save sets them to what the page shows.
    fn adopt_graphing(&mut self) {
        let g = self.screens.config().graphing;
        if g != self.graphing {
            self.shell.model.graph = crate::graph::GraphSettings::from_config(&g);
            self.graphing = g;
            self.sync_model();
        }
    }

    /// Refresh the upmixer's telemetry, at most once a second and only while
    /// its panel is the one on screen.
    ///
    /// `REQ_UPMIX_GET_STATUS` is a 16-byte structure with no scalar readback,
    /// so it goes through the transport rather than the registry, the way the
    /// preset directory does.
    fn poll_upmix_status(&mut self, session: &mut Session, now: Instant) {
        let showing = matches!(self.shell.tool, Some((Tool::Upmixer, _)));
        if !showing || !panel::has_feature(&self.state, "upmixer") {
            return;
        }
        if now.duration_since(self.last_upmix_poll) < Duration::from_secs(1) {
            return;
        }
        self.last_upmix_poll = now;
        let read = session.with_transport(|t| {
            t.control_in(
                dspi_proto::generated::opcodes::REQ_UPMIX_GET_STATUS,
                0,
                dspi_proto::packets::UpmixStatus::SIZE as u16,
            )
        });
        if let Ok(bytes) = read
            && let Ok(status) = dspi_proto::packets::UpmixStatus::decode(&bytes)
        {
            self.state.upmix_status = Some(status);
        }
    }

    /// Read the limiters' gain reduction (`REQ_LIMITER` index
    /// `LIMITER_GET_METER`, limiter.h:19) on every meter tick, but only while
    /// an output page is showing and some limiter is on: the Console's
    /// `pollLimiter(meter:)`, which clears the readings once rather than
    /// polling when nothing can be limiting (Commands.swift:1713-1728).
    fn poll_limiter_meter(&mut self, session: &mut Session) {
        let showing = self.shell.tool.is_none()
            && self.shell.settings.is_none()
            && matches!(self.shell.model.selection, Selection::Output(_));
        if !panel::has_feature(&self.state, screens::limiter::FEATURE) {
            return;
        }
        if !showing || !screens::limiter::any_on(&self.state) {
            self.state.limiter_meter = None;
            return;
        }
        let _ = self.state.refresh_limiter_meter(session);
    }

    /// The Subharmonic Synthesizer's runtime state, while its panel is on
    /// screen: the headroom cost (`REQ_GET_SUBHARM_HEADROOM`, config.h:194)
    /// when it opens and whenever the section changes, which is after every
    /// write that can move it and after another host's too; solo
    /// (`REQ_GET_SUBHARM_SOLO`, config.h:202) once a second, since nothing
    /// notifies it; and the sub meters (`REQ_GET_SUBHARM_METER`,
    /// config.h:200) at the Console's 10 Hz while the module is on.
    ///
    /// When the panel has gone, by whatever route, solo is switched off if it
    /// was on, as the Console's window does on close: solo mutes the program
    /// on the masked outputs, and the firmware keeps it across a preset load.
    fn poll_subharm(&mut self, session: &mut Session, now: Instant) {
        let showing = matches!(self.shell.tool, Some((Tool::Subharm, _)))
            && panel::has_feature(&self.state, "subharmonic_synth");
        if !showing {
            if std::mem::take(&mut self.subharm.open) {
                self.subharm.seen = None;
                if self.state.subharm_solo == Some(true) {
                    self.set(session, "sub.solo", &[], Value::Bool(false));
                }
            }
            return;
        }
        let opened = !std::mem::replace(&mut self.subharm.open, true);
        let section = self.state.subharm();
        if self.subharm.seen != Some(section) {
            self.subharm.seen = Some(section);
            let _ = self.state.refresh_subharm_headroom(session);
        }
        if opened || now.duration_since(self.subharm.solo_at) >= Duration::from_secs(1) {
            self.subharm.solo_at = now;
            let _ = self.state.refresh_subharm_solo(session);
        }
        if section.enabled && now.duration_since(self.subharm.meter_at) >= SUBHARM_METER_PERIOD {
            self.subharm.meter_at = now;
            let _ = self.state.refresh_subharm_meter(session);
        }
    }

    /// Refresh the Stats panel's diagnostics, every two seconds and only while
    /// that panel is on screen.
    ///
    /// None of it is in the bulk packet and none of it is notified, so it can
    /// only be asked for; two dozen control transfers every two seconds is
    /// worth it for a panel someone is reading and worth nothing otherwise.
    /// The screen on top gets its once-a-second poll.
    fn poll_screen(&mut self, session: &mut Session, now: Instant) {
        if now.duration_since(self.last_screen_poll) < Duration::from_secs(1) {
            return;
        }
        self.last_screen_poll = now;
        if let Some(s) = self.shell.settings.as_deref_mut() {
            s.poll(session, &self.state);
            self.refresh_cs_aux(session);
        } else if let Some((_, s)) = self.shell.tool.as_mut() {
            s.poll(session, &self.state);
        }
    }

    /// Re-read every auxiliary output's live state and level while Settings
    /// is open, for its Auxiliary Outputs page: on open and on each poll, with
    /// `NOTIFY_EVT_CS_AUX` keeping it current in between. Only a device with
    /// aux outputs is asked (caps v18, control_surfaces.h:141-146).
    fn refresh_cs_aux(&mut self, session: &mut Session) {
        use crate::settings::cs_model::{CsCaps, aux_supported};
        let supported = self
            .state
            .caps
            .cs
            .as_ref()
            .is_some_and(|c| aux_supported(&CsCaps::from(c)));
        if supported {
            let _ = self.state.refresh_cs_aux(session);
        }
    }

    fn poll_stats(&mut self, session: &mut Session, now: Instant) {
        if !matches!(self.shell.tool, Some((Tool::Stats, _)))
            || now.duration_since(self.last_stats_poll) < Duration::from_secs(2)
        {
            return;
        }
        self.last_stats_poll = now;
        let previous = self.shared.borrow().stats.clone();
        let stats = actions::read_stats(session, &self.state, &previous);
        self.shared.borrow_mut().stats = stats;
    }

    fn on_meters(&mut self, m: dspi_session::Meters, now: Instant, session: &mut Session) {
        let clips = m.clipped.iter().enumerate().fold(0u32, |acc, (i, &c)| {
            let bit = u32::try_from(i).ok().and_then(|i| 1u32.checked_shl(i));
            if c { acc | bit.unwrap_or(0) } else { acc }
        });
        let update = self.meters.update(now, &m.peaks, clips);
        self.state.update_meters(m);
        // The clip marker is the shared clip hold: the Console's, ten seconds
        // after the last new clip, when the device is asked to forget too. A
        // channel still clipping lights again on the next read; where the
        // clear is refused, the marker follows the device's sticky bits.
        if update.clear_due {
            let sent = session.write("meters.clear", &[], Value::Trigger);
            self.meters.clear_result(now, sent.is_ok());
        }
        self.state.clip_latched = self.meters.clip_hold();
    }

    /// Meters, notifications, status expiry, peak ballistics.
    pub fn tick(&mut self, session: &mut Session, notifications: Option<&Notifications>) {
        let now = Instant::now();

        match session.meters() {
            Ok(m) => self.on_meters(m, now, session),
            // The control path is the other witness to a disconnect.
            Err(dspi_session::WriteError::Transport(
                dspi_transport::TransportError::Disconnected,
            )) => {
                self.state.connected = false;
            }
            Err(_) => {}
        }
        self.poll_upmix_status(session, now);
        self.poll_limiter_meter(session);
        self.poll_subharm(session, now);
        self.poll_stats(session, now);
        self.poll_screen(session, now);
        self.tick_analyser(session, now);

        self.absorb_notes(session, notifications, now);
        if self
            .devices_checked
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(5))
        {
            self.refresh_devices();
        }
        if let Some(until) = self.status_until
            && now >= until
        {
            self.shell.model.status = None;
            self.status_until = None;
        }
        self.advance_boot();
        self.advance_rebuild();
        // Losing the device takes a channel page back to the overview, as
        // the Console does (`ContentView.swift:964-972`): the sidebar row
        // that would close the page has gone with the device.
        // -- WP TM8 -- A hub being reconnected to leaves the page readable.
        if !self.state.connected
            && !self.link_reconnecting()
            && self.shell.model.selection != Selection::Overview
        {
            self.select(Selection::Overview);
        }
        self.sync_model();
    }

    /// Fold one notification into the state, and note what it asks of the
    /// runner in `fx`.
    fn take_note(&mut self, note: &Notification, now: Instant, fx: &mut NoteEffects) {
        let at = now.duration_since(self.started).as_secs_f64();
        // The monitor is fed from this drain rather than from a reader of its
        // own: two readers on one endpoint would each see half the events,
        // and the log has to keep running while the panel is closed so
        // opening it shows what just happened.
        self.shared.borrow_mut().log.push(at, note);
        // -- WP TM5 --
        self.shared.borrow_mut().spectrum.note(
            note,
            self.link_kind,
            self.link_session,
            &self.state,
        );
        // The source byte alone does not say whose a change is: that depends on
        // the link (PLAN §1.2).
        let own = self
            .echo
            .classify_notification(self.link_kind, self.link_session, note);
        match self.state.apply(note) {
            Applied::Section { name, source } => {
                if !own {
                    let words = connect::foreign_words(self.link_kind, source, note.origin);
                    fx.changed_elsewhere = Some((name, words));
                    if matches!(name, "user_volume" | "master_volume")
                        && let Some(from) = self.shown_volume
                    {
                        self.ease_from = Some((now, from));
                    }
                }
            }
            Applied::NeedsReread { source } => {
                fx.reread = true;
                fx.preset_bulk |= source == Source::Preset;
                // A re-read forced by a lost packet is no change at all.
                if !own
                    && matches!(note.event, dspi_session::Event::BulkInvalidated { .. })
                    && connect::bulk_is_news(self.link_kind, self.link_session, source, note.origin)
                {
                    let words = connect::foreign_words(self.link_kind, source, note.origin);
                    fx.changed_elsewhere = Some(("settings", words));
                }
            }
            Applied::PresetLoaded { slot } => {
                fx.reread = true;
                fx.loaded = Some(slot);
            }
            Applied::InputFormat { channels } => {
                self.note(format!("{channels} input channels active"));
            }
            // The ADAT link changing state (NOTIFY_EVT_ADAT_STATE and
            // NOTIFY_EVT_ADAT_INPUT_STATE, notify.h) moves the Stats panel's
            // ADAT sections on the next tick rather than at the next
            // two-second poll.
            Applied::Status(
                dspi_session::notify::Event::AdatState { .. }
                | dspi_session::notify::Event::AdatInputState { .. },
            ) => self.last_stats_poll = now - Duration::from_secs(3),
            Applied::Status(_) | Applied::Nothing => {}
        }
    }

    /// Everything the device has said since the last look: fold it in,
    /// re-read when it asks, and take a preset it loaded as the new baseline.
    ///
    /// Run on every tick, and before every write of ours, so a preset loaded
    /// elsewhere becomes the baseline before an edit of ours can land on top
    /// of it and be counted as saved.
    fn absorb_notes(
        &mut self,
        session: &mut Session,
        notifications: Option<&Notifications>,
        now: Instant,
    ) {
        let mut fx = NoteEffects::default();
        if let Some(n) = notifications {
            self.shared.borrow_mut().log.active = true;
            for note in n.drain() {
                self.take_note(&note, now, &mut fx);
            }
            if n.is_disconnected() {
                self.state.connected = false;
            }
        }
        let mut fresh = false;
        if fx.reread || self.state.stale {
            // A failed read leaves the shadow untrusted, so the next tick
            // tries again.
            fresh = self.refresh(session);
            self.state.stale = !fresh;
        }
        // A preset the device loaded is the new baseline, as a preset this
        // host loaded is; otherwise the marker compares against the old one.
        // A slot that fails its CRC check still announces itself, but the
        // device keeps its active slot and its live state
        // (flash_storage.c:4316-4327), so the slot is taken from the device.
        match fx.loaded {
            Some(slot) if fresh => {
                let active = read_active_slot(session);
                self.state.caps.active_preset = active.or(Some(slot));
                if active.is_none_or(|a| a == slot) {
                    self.state.mark_saved();
                    self.note(format!("Preset {} loaded", slot as u16 + 1));
                } else {
                    self.note(format!("Preset {} failed to load", slot as u16 + 1));
                }
            }
            None if fresh && fx.preset_bulk => self.state.mark_saved(),
            _ => {}
        }
        if let Some((name, words)) = fx.changed_elsewhere {
            self.echo(format!("{} {words}", name.replace('_', " ")));
        }
    }

    /// Fold in what the reader has queued, before a write of ours.
    fn absorb_pending_notes(&mut self, session: &mut Session) {
        if let Some(n) = self.notes.take() {
            self.absorb_notes(session, Some(&n), Instant::now());
            self.notes = Some(n);
        }
    }

    /// Wait for a deferred device action to finish: the device answers the
    /// request at once and does the work later in its main loop, ending with
    /// a `BULK_INVALIDATED` from `source` (notify.c:235-247).
    ///
    /// The notifications that arrive meanwhile are folded in as a tick would,
    /// and the rest stay queued for the tick. Without a notification reader
    /// there is nothing to wait on, so this waits as long as the Console does
    /// after a load (Commands.swift:4196).
    fn wait_for_bulk(&mut self, source: Source) -> bool {
        let Some(n) = self.notes.take() else {
            std::thread::sleep(DEFERRED_SETTLE);
            return false;
        };
        let deadline = Instant::now() + DEFERRED_TIMEOUT;
        let mut fx = NoteEffects::default();
        let mut done = false;
        while let Some(left) = deadline.checked_duration_since(Instant::now())
            && let Some(note) = n.next(left)
        {
            done = matches!(
                note.event,
                dspi_session::notify::Event::BulkInvalidated { source: s } if s == source
            );
            self.take_note(&note, Instant::now(), &mut fx);
            if done {
                break;
            }
        }
        self.notes = Some(n);
        done
    }

    /// Wait for the device to report `slot` as its active preset, which is
    /// how a deferred save is seen to be taken: `preset_save` makes the slot
    /// it wrote the active one (flash_storage.c:4295-4297). The Console waits
    /// the same way between the two saves of a copy
    /// (`waitForPresetActivation`, Commands.swift:4213-4225).
    ///
    /// A device that cannot be asked is given the settle time instead.
    fn wait_for_active(&mut self, session: &mut Session, slot: u8) -> bool {
        let deadline = Instant::now() + DEFERRED_TIMEOUT;
        loop {
            match read_active_slot(session) {
                Some(a) if a == slot => return true,
                None => {
                    std::thread::sleep(DEFERRED_SETTLE);
                    return true;
                }
                Some(_) if Instant::now() >= deadline => return false,
                Some(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    /// Save the live state into `slot`, and wait for the device to take it.
    ///
    /// The device keeps one pending save (vendor_commands.c:2679-2694) and
    /// takes it on a later pass of its main loop (main.c:3058-3067), so a
    /// second save sent before then replaces the first, which is never
    /// written. A save of the slot already active cannot be seen landing by
    /// the active slot, so it is given the settle time.
    fn save_slot(&mut self, session: &mut Session, slot: u8) -> bool {
        let already = read_active_slot(session) == Some(slot);
        match session.write("preset.save", &[slot], Value::Trigger) {
            Ok(Outcome::Rejected { .. }) | Err(_) => return false,
            Ok(_) => {}
        }
        if already {
            std::thread::sleep(DEFERRED_SETTLE);
            true
        } else {
            self.wait_for_active(session, slot)
        }
    }

    /// Read the spectrum analyser's caps, and push the Settings page's
    /// values on the next tick: the device forgets them at every power cycle.
    /// Called on connect and after a device switch.
    pub fn connect_analyser(&mut self, session: &mut Session) {
        let absent = self
            .state
            .caps
            .features
            .iter()
            .any(|f| f.name == "spectrum_analyser" && !f.present);
        let mut app = self.shared.borrow_mut();
        // -- WP TM5 --
        app.spectrum.new_connection();
        let engine = &mut app.spectrum.engine;
        if absent {
            engine.disconnect();
        } else {
            let _ = session.with_transport(|t| Ok(engine.connect(t)));
        }
    }

    // -- WP TM5 --
    /// `f`: the live spectrum on the response graph.
    fn toggle_spectrum_overlay(&mut self, session: &mut Session) {
        if !self.analyser_ready(session) {
            return;
        }
        let page = self.analyser_page();
        let on = self.shared.borrow_mut().spectrum.toggle_overlay(page);
        self.note(format!(
            "Spectrum on the graph {}",
            if on { "on" } else { "off" }
        ));
    }

    /// Whether the analyser can be used, connecting it if it has not been;
    /// says so when it cannot.
    fn analyser_ready(&mut self, session: &mut Session) -> bool {
        if !self.shared.borrow().spectrum.engine.supported() {
            self.connect_analyser(session);
        }
        let ready = self.shared.borrow().spectrum.engine.supported();
        if !ready {
            self.note("This firmware has no spectrum analyser.".to_string());
        }
        ready
    }

    /// The page's channel, as the analyser taps it.
    fn analyser_page(&self) -> Option<(u8, u8)> {
        use dspi_session::rta::{TAP_INPUT, TAP_OUTPUT};
        match self.shell.model.selection {
            Selection::Overview => None,
            Selection::Input(i) => Some((TAP_INPUT, i as u8)),
            Selection::Output(o) => Some((TAP_OUTPUT, o as u8)),
        }
    }

    // -- WP TM6 --
    /// `F`: the third-octave bars above the page.
    fn toggle_spectrum_bars(&mut self, session: &mut Session) {
        if !self.analyser_ready(session) {
            return;
        }
        let page = self.analyser_page();
        let on = self.shared.borrow_mut().spectrum.toggle_bars(page);
        let n = self.bars_channels();
        self.note(match (on, self.shell.bars_fit(n)) {
            (false, _) => "Spectrum bars off".to_string(),
            (true, true) => "Spectrum bars on".to_string(),
            (true, false) => "Spectrum bars on, but there is no room: = shrinks the graph".into(),
        });
    }

    // -- WP TM6 --
    /// How many channels the bars draw.
    fn bars_channels(&self) -> usize {
        let app = self.shared.borrow();
        screens::spectrum::selection(&self.state, &app.spectrum)
            .channels
            .len()
    }

    // -- WP TM5 --
    /// `t`: push this console's analyser configuration over the one being
    /// followed. Does nothing while driving.
    fn take_analyser_control(&mut self) {
        let mut app = self.shared.borrow_mut();
        if app.spectrum.sharing.following().is_some() {
            app.spectrum.sharing.take_control();
            drop(app);
            self.note("Took control of the analyser".to_string());
        }
    }

    /// One analyser poll, inside its budget, after the meters so they are
    /// never late for it. Skipped outright when a key is waiting.
    fn tick_analyser(&mut self, session: &mut Session, now: Instant) {
        // -- WP TM5 --
        let shown = self.shell.graph_on_screen();
        self.shared.borrow_mut().spectrum.set_overlay_shown(shown);
        // -- WP TM6 --
        let shown = self.shell.bars_on_screen(self.bars_channels());
        self.shared.borrow_mut().spectrum.set_bars_shown(shown);
        if self.input_pending {
            return;
        }
        let report = {
            let mut app = self.shared.borrow_mut();
            // -- WP TM5 --
            let spectrum = &mut app.spectrum;
            spectrum.sync(&self.state);
            session
                .with_transport(|t| Ok(spectrum.tick(t, now)))
                .unwrap_or_default()
        };
        if report.disconnected {
            self.state.connected = false;
        }
    }

    /// Move the bootloader handoff along, and close it out when it lands.
    fn advance_boot(&mut self) {
        let Some(watch) = self.boot.as_mut() else {
            return;
        };
        watch.poll(&actions::RealBootProbe);
        let (status, fraction, finished) = (watch.status(), watch.fraction(), watch.finished());
        if let Some((AppDialog::BootWait, dialog)) = self.dialog.as_mut() {
            dialog.set_progress(fraction, status.clone());
            if finished {
                // A progress dialog has no buttons, so the wait becomes a
                // report the person can dismiss once it has an answer.
                *dialog = Dialog::report("Reboot into Bootloader", vec![status]);
            }
        }
        if finished {
            self.dialog = self.dialog.take().map(|(_, d)| (AppDialog::Report, d));
            self.boot = None;
        }
    }

    /// Move the AutoEQ rebuild's progress dialog along.
    fn advance_rebuild(&mut self) {
        let Some(handle) = self.rebuild.as_mut() else {
            return;
        };
        let progress = handle.poll().clone();
        if let Some((AppDialog::AutoEqRebuildProgress, dialog)) = self.dialog.as_mut() {
            dialog.set_progress(progress.fraction, progress.status.clone());
        }
        let Some(done) = progress.done else { return };
        self.rebuild = None;
        self.dialog = None;
        match done {
            // -- WP TM12 -- A note lasts three seconds: what is missing stays up.
            Ok(n) if n.failed_profiles > 0 || n.failed_listings > 0 => {
                self.reload_autoeq();
                self.dialog = Some((
                    AppDialog::Report,
                    Dialog::report(
                        "Database rebuilt with parts missing",
                        screens::autoeq::rebuild_report(&n),
                    ),
                ));
            }
            Ok(n) => {
                self.reload_autoeq();
                self.note(format!("Database rebuilt successfully! Entries: {n}"));
            }
            Err(e) => self.note(format!("Rebuild failed: {e}")),
        }
    }

    pub fn draw(&mut self, area: Rect, buf: &mut Buffer) {
        self.shell.draw(area, buf, &self.state);
        let theme = self.shell.theme.clone();
        if let Some((_, p)) = &self.popup {
            let (w, h) = p.size(area.width.saturating_sub(4), area.height.saturating_sub(4));
            let r = Rect::new(
                area.x + (area.width - w) / 2,
                area.y + (area.height - h) / 2,
                w,
                h,
            );
            p.draw(r, buf, &theme);
        }
        if let Some((_, d)) = &self.dialog {
            let r = d.size(area);
            d.draw(r, buf, &theme);
        }
        if let Some(p) = &self.prompt {
            p.draw(area, buf, &theme);
        }
    }
}

/// Run the live interface until the person quits.
pub fn run(mut live: Live, session: &mut Session) -> io::Result<()> {
    live.arm_notifications(session);
    live.connect_analyser(session);
    live.refresh_devices();
    let perf = live.perf;
    live.refresh_presets(session);
    // -- WP TM26 -- An error, not a panic, when there is no terminal to take
    // over (hw-1 D1).
    let mut terminal = ratatui::try_init()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot open the terminal: {e}")))?;
    let mut last_poll = Instant::now() - perf.meter_interval;

    let result = (|| -> io::Result<()> {
        loop {
            if last_poll.elapsed() >= perf.meter_interval {
                live.input_pending = event::poll(Duration::ZERO)?;
                let notes = live.notes.take();
                live.tick(session, notes.as_ref());
                live.notes = notes;
                last_poll = Instant::now();
            }
            terminal.draw(|f| {
                let area = f.area();
                live.draw(area, f.buffer_mut());
            })?;
            if event::poll(perf.event_timeout)?
                && let TermEvent::Key(key) = event::read()?
                && key.kind == event::KeyEventKind::Press
            {
                // Ctrl-C always leaves, even with a dialog up.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    live.should_quit = true;
                }
                live.handle_key(session, key);
            }
            // -- WP UTM -- (UXP A13) a shell verb from the `:` line runs with the
            // terminal handed back, so its output and its questions are the shell's
            if let Some(words) = live.shell_run.take() {
                live.flush_writes(session, true);
                ratatui::restore();
                let code = live.run_shell_verb(session, &words);
                back_to_the_interface(code)?;
                terminal = ratatui::try_init()?;
                terminal.clear()?;
            }
            // The event wait is shorter than the write interval, so a held
            // write goes out on time.
            live.flush_writes(session, false);
            if let Some(serial) = live.switch_to.take() {
                live.release(session);
                // Nothing about the old device is carried over: a different
                // unit can have a different channel count, a different firmware
                // and different features, and reusing any of that would show
                // one device's shape under another's name.
                let target = Target::Usb {
                    serial: Some(serial.clone()),
                };
                // -- WP TMCORE -- through the embedder's opener
                match live.open_target(&target) {
                    Ok(next) => {
                        *session = next.session;
                        live.adopt(session, next.state);
                        live.set_protocol(next.protocol); // -- WP PGTM --
                        live.set_connection(&target, &next.ext);
                        live.arm_notifications(session);
                        live.connect_analyser(session);
                    }
                    Err(e) => live.note(format!("Could not open {serial}: {e}")),
                }
            }
            live.advance_reconnect(session);
            if live.should_quit {
                live.release(session);
                return Ok(());
            }
        }
    })();
    ratatui::restore();
    result
}

// -- WP UTM --
/// After a shell verb: wait for Enter, so its output can be read before the
/// interface covers it.
fn back_to_the_interface(code: u8) -> io::Result<()> {
    use std::io::Write;
    let mut out = io::stdout();
    if code != 0 {
        writeln!(out, "(exit code {code})")?;
    }
    write!(out, "\nPress Enter to go back to the interface ")?;
    out.flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

/// The connect animation: the curve draws left to right over `REVEAL`, the
/// rest of it hidden as non-finite points the graph skips.
fn reveal(mut points: Vec<f64>, started: Instant, animate: bool) -> Vec<f64> {
    if !animate {
        return points;
    }
    let t = started.elapsed().as_secs_f64() / REVEAL.as_secs_f64();
    if t >= 1.0 {
        return points;
    }
    let shown = (t * points.len() as f64) as usize;
    for p in points.iter_mut().skip(shown) {
        *p = f64::NAN;
    }
    points
}

/// A value in the words the registry uses for it: a choice by name, a number
/// with its unit.
fn display_value(d: &dspi_proto::registry::ParamDesc, v: &Value) -> String {
    if let (dspi_proto::registry::Kind::Choice(variants), Some(n)) = (d.kind, v.as_u8())
        && let Some((_, name)) = variants.iter().find(|(raw, _)| *raw == n)
    {
        return (*name).to_string();
    }
    v.display(d.kind.unit())
}

#[cfg(test)]
mod tests_wp_r1 {
    use super::*;
    use crate::theme::{ColorDepth, Glyphs};
    use dspi_proto::generated::notify as nfy;
    use dspi_session::notify::Event;
    use dspi_transport::{FakeDevice, FakeRemote, Transport, TransportKind};
    use ratatui::buffer::Buffer;

    /// A live interface on a demo device, with its remote control.
    fn on_demo() -> (Live, Session, FakeRemote) {
        let fake = FakeDevice::demo();
        let remote = fake.remote();
        let mut t: Box<dyn Transport> = Box::new(fake);
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let ext = ExtState::probe(t.as_mut()).unwrap();
        let mut session = Session::new(t, caps.clone()).unwrap();
        let bulk = session.snapshot().unwrap();
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut live = Live::new(
            DeviceState::new(caps, bulk),
            theme,
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        live.set_connection(&Target::Demo, &ext);
        live.arm_notifications(&mut session);
        (live, session, remote)
    }

    fn note(event: Event, origin: Option<u16>) -> Notification {
        Notification {
            seq: 1,
            event,
            lost: false,
            origin,
            received: Instant::now(),
        }
    }

    fn param(source: Source, origin: Option<u16>) -> Notification {
        let offset = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "master_volume")
            .map(|(_, o, _)| *o as u16)
            .unwrap_or_default();
        note(
            Event::ParamChanged {
                offset,
                source,
                bytes: (-20.0f32).to_le_bytes().to_vec(),
            },
            origin,
        )
    }

    /// What the echo line says after this one notification.
    fn line(l: &mut Live, s: &mut Session, n: &Notification) -> Option<String> {
        let mut fx = NoteEffects::default();
        l.take_note(n, Instant::now(), &mut fx);
        let _ = s;
        fx.changed_elsewhere
            .map(|(name, words)| format!("{name} {words}"))
    }

    #[test]
    fn wp_r1_the_changed_elsewhere_line_follows_the_link() {
        let (mut l, mut s, _r) = on_demo();
        // USB and the demo device: HOST_SET is our own echo.
        l.link_kind = TransportKind::Usb;
        assert_eq!(line(&mut l, &mut s, &param(Source::HostSet, None)), None);
        // A BULK_SET with no write of ours pending is not ours.
        assert_eq!(
            line(&mut l, &mut s, &param(Source::BulkSet, None)).as_deref(),
            Some("master_volume changed elsewhere")
        );
        assert_eq!(
            line(&mut l, &mut s, &param(Source::Gpio, None)).as_deref(),
            Some("master_volume changed by control surface")
        );

        // MIDI: our writes come back as MIDI, and a HOST_SET is another host.
        l.link_kind = TransportKind::Midi;
        assert_eq!(line(&mut l, &mut s, &param(Source::Midi, None)), None);
        assert_eq!(
            line(&mut l, &mut s, &param(Source::HostSet, None)).as_deref(),
            Some("master_volume changed over USB")
        );

        // Link: the origin decides, whatever the source.
        l.link_kind = TransportKind::Link;
        l.link_session = Some(7);
        assert_eq!(
            line(&mut l, &mut s, &param(Source::Uart, Some(7))),
            None,
            "a bridge writes for us over UART"
        );
        assert_eq!(
            line(&mut l, &mut s, &param(Source::HostSet, Some(8))).as_deref(),
            Some("master_volume changed by another Link client")
        );
        assert_eq!(
            line(&mut l, &mut s, &param(Source::HostSet, Some(0))).as_deref(),
            Some("master_volume changed elsewhere")
        );
    }

    /// PLAN §1.2's "Bulk echoes": the terminal writes no bulk, so nothing of
    /// ours is ever outstanding and the filter claims nothing, on either mode
    /// of firmware. Every invalidation is re-read. The line is said only when
    /// it is true: a `HOST_SET` can be our own big scalar write, an `UNKNOWN`
    /// a merge holding ours.
    #[test]
    fn wp_r1_every_bulk_invalidation_is_re_read_and_only_a_foreign_one_is_news() {
        let (mut l, _s, _r) = on_demo();
        l.link_kind = TransportKind::Usb;
        for tagged in [false, true] {
            l.echo.set_bulk_source(tagged);
            for (source, news) in [
                (Source::HostSet, false),
                (Source::Unknown, false),
                (Source::BulkSet, true),
                (Source::Midi, true),
            ] {
                let n = note(Event::BulkInvalidated { source }, None);
                let mut fx = NoteEffects::default();
                l.take_note(&n, Instant::now(), &mut fx);
                assert!(fx.reread, "{source:?} is re-read (tagged: {tagged})");
                assert_eq!(fx.changed_elsewhere.is_some(), news, "{source:?}");
            }
        }
    }

    #[test]
    fn wp_r1_a_preset_load_or_a_lost_packet_is_not_reported_as_a_change_elsewhere() {
        let (mut l, _s, _r) = on_demo();
        for source in [Source::Preset, Source::Factory] {
            let n = note(Event::BulkInvalidated { source }, None);
            let mut fx = NoteEffects::default();
            l.take_note(&n, Instant::now(), &mut fx);
            assert!(fx.reread);
            assert!(fx.changed_elsewhere.is_none(), "{source:?}");
        }
        let mut lost = note(Event::Idle, None);
        lost.lost = true;
        let mut fx = NoteEffects::default();
        l.take_note(&lost, Instant::now(), &mut fx);
        assert!(fx.reread);
        assert!(fx.changed_elsewhere.is_none());
    }

    #[test]
    fn wp_r1_a_foreign_bulk_says_so_on_the_echo_line() {
        let (mut l, mut s, r) = on_demo();
        // The demo device's `load_image` is a bulk apply by somebody else.
        r.load_image(r.image(), nfy::PARAM_SRC_BULK_SET as u8);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let notes = l.notes.take();
            l.tick(&mut s, notes.as_ref());
            l.notes = notes;
            if !l.shell.model.echo.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(l.shell.model.echo, "settings changed elsewhere");
    }

    #[test]
    fn wp_r1_the_title_bar_names_the_link_and_the_extensions() {
        let (mut l, _s, _r) = on_demo();
        l.sync_model();
        let m = &l.shell.model;
        assert_eq!(m.via, "demo");
        assert!(m.ext.starts_with("ext "), "{:?}", m.ext);
        let area = Rect::new(0, 0, 130, 40);
        let mut buf = Buffer::empty(area);
        l.draw(area, &mut buf);
        let top: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(top.contains("· demo · ext "), "{top}");
        assert!(top.contains("Connected"), "{top}");
    }

    #[test]
    fn wp_r1_a_narrow_title_bar_keeps_the_connection_state() {
        let (mut l, _s, _r) = on_demo();
        l.sync_model();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        l.draw(area, &mut buf);
        let top: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(top.contains("Connected"), "{top}");
    }

    /// Unplug, and the interface goes offline and comes back on the same demo
    /// target by itself: the reader saw the end, the transport was let go, and
    /// the device was probed again from scratch.
    #[test]
    fn wp_r1_a_lost_device_comes_back_without_a_restart() {
        let (mut l, mut s, r) = on_demo();
        r.disconnect();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut went_offline = false;
        while Instant::now() < deadline {
            let notes = l.notes.take();
            l.tick(&mut s, notes.as_ref());
            l.notes = notes;
            went_offline |= !l.state.connected;
            l.advance_reconnect(&mut s);
            if went_offline && l.state.connected {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(went_offline, "the disconnect was seen");
        assert!(l.state.connected, "and the device came back");
        assert!(l.shell.model.connected);
        assert!(l.notes.is_some(), "with its notifications armed again");
        assert!(!l.offline);
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|m| m.contains("Reconnected")),
            "{:?}",
            l.shell.model.status
        );
        // And it answers.
        assert!(s.meters().is_ok());
    }

    /// A token the reconnect's re-pairing could not store is said on screen:
    /// its thread must not write to the terminal the interface is drawing on.
    #[test]
    fn wp_r1_a_failed_token_save_during_a_reconnect_reaches_the_status_line() {
        use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap();
        // A file where the token file's directory should be.
        let blocked = connect::test_dir().join(format!("live-blocked-{}", std::process::id()));
        std::fs::write(&blocked, "x").unwrap();
        let mut link = connect::LinkTarget::parse(&hub.addr().to_string()).unwrap();
        link.pin = Some("1234".into());
        link.tokens = Some(blocked.join("tokens.toml"));

        let (mut l, mut s, _r) = on_demo();
        // `set_connection` holds the target to the device it has.
        l.state.caps.serial = HubConfig::default().serial;
        l.set_connection(&Target::Link(link), &ExtState::default());
        l.state.connected = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !l.state.connected && Instant::now() < deadline {
            l.advance_reconnect(&mut s);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(l.state.connected, "the device came back");
        let status = l.shell.model.status.clone().unwrap_or_default();
        assert!(status.contains("Reconnected"), "{status:?}");
        assert!(status.contains("pairing token"), "{status:?}");
    }

    // -- WP TM9 --
    /// What a reconnect reached is what the next one starts from.
    #[test]
    fn wp_tm9_the_interface_keeps_the_target_a_reconnect_reached() {
        use dspi_link::fakehub::{FakeHub, HubConfig};
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut link = connect::LinkTarget::parse(&hub.addr().to_string()).unwrap();
        let tokens = connect::test_dir().join(format!("live-tm9-{}.toml", std::process::id()));
        link.tokens = Some(tokens);
        assert_eq!(link.hub_id, None);

        let (mut l, mut s, _r) = on_demo();
        l.state.caps.serial = HubConfig::default().serial;
        l.set_connection(&Target::Link(link), &ExtState::default());
        l.state.connected = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !l.state.connected && Instant::now() < deadline {
            l.advance_reconnect(&mut s);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(l.state.connected, "the device came back");
        let Some(Target::Link(kept)) = &l.target else {
            panic!("still a hub");
        };
        assert_eq!(kept.hub_id, Some(HubConfig::default().hub_id));
        assert_eq!(kept.device, Some(HubConfig::default().serial));
    }

    /// PLAN §1.7: a device's text cannot reach the terminal raw, because every
    /// cell goes through ratatui's buffer, which drops control characters.
    #[test]
    fn wp_r1_the_interface_draws_no_control_characters_from_a_device() {
        let (mut l, _s, _r) = on_demo();
        l.state.caps.serial = "\u{1b}[2J\u{9b}2J\u{7}evil".into();
        l.sync_model();
        let area = Rect::new(0, 0, 130, 40);
        let mut buf = Buffer::empty(area);
        l.draw(area, &mut buf);
        assert!(
            !buf.content()
                .iter()
                .any(|c| c.symbol().contains(char::is_control))
        );
    }

    #[test]
    fn wp_r1_without_a_target_nothing_tries_to_reconnect() {
        let (mut l, mut s, _r) = on_demo();
        l.reconnect = None;
        l.target = None;
        l.state.connected = false;
        l.advance_reconnect(&mut s);
        assert!(!l.offline);
        assert!(!l.state.connected);
    }

    #[test]
    fn wp_r1_a_handoff_to_the_bootloader_is_not_interrupted() {
        let (mut l, mut s, _r) = on_demo();
        l.boot = Some(actions::BootloaderWatch::new());
        l.state.connected = false;
        l.advance_reconnect(&mut s);
        assert!(l.offline, "the transport is let go");
        assert!(!l.state.connected, "but the watch runs first");
    }
}

// -- WP TM8 --
/// The owner's rule for a lost hub, against a fake hub: the status line and its
/// key, the mirror still readable, and a failed write gone for good.
#[cfg(test)]
mod tests_wp_tm8 {
    use super::*;
    use crate::theme::{ColorDepth, Glyphs};
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};

    const LONG: Duration = Duration::from_secs(10);

    fn disconnect_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
    }

    /// A live interface on a fake hub, as `uconsole --link` starts one.
    fn on_hub(name: &str) -> (Live, Session, FakeHub) {
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap();
        let dir = connect::test_dir().join(format!("tm8-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut link = connect::LinkTarget::parse(&hub.addr().to_string()).unwrap();
        link.tokens = Some(dir.join("tokens.toml"));
        let target = Target::Link(link);
        let prompt = |_: &str, _: &str| Some("1234".to_string());
        let c = target.connect(&prompt).unwrap();
        let mut session = c.session;
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut live = Live::new(
            c.state,
            theme,
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        live.set_connection(&target, &c.ext);
        live.arm_notifications(&mut session);
        (live, session, hub)
    }

    /// One turn of `run`'s loop, without the terminal.
    fn turn(l: &mut Live, s: &mut Session) {
        let notes = l.notes.take();
        l.tick(s, notes.as_ref());
        l.notes = notes;
        l.flush_writes(s, false);
        l.advance_reconnect(s);
        std::thread::sleep(Duration::from_millis(10));
    }

    fn turn_until(l: &mut Live, s: &mut Session, what: &str, done: impl Fn(&Live) -> bool) {
        let deadline = Instant::now() + LONG;
        while !done(l) {
            assert!(Instant::now() < deadline, "never: {what}");
            turn(l, s);
        }
    }

    /// The hub cuts the session and will not take another until told.
    fn lose(l: &mut Live, s: &mut Session, hub: &FakeHub) {
        hub.refuse_handshake(Some(503));
        hub.drop_connections();
        turn_until(l, s, "the loss is seen", |l| l.offline);
    }

    fn frame(l: &mut Live, w: u16, h: u16) -> String {
        crate::render_frame(w, h, |a, b| l.draw(a, b))
    }

    /// Commands the hub's device was asked to change something with.
    fn writes(hub: &FakeHub) -> usize {
        hub.log()
            .iter()
            .filter(|e| e.1 == dspi_link::Dir::Set as u8)
            .count()
    }

    #[test]
    fn wp_tm8_the_status_line_uses_the_shared_words_on_every_screen() {
        let (mut l, mut s, hub) = on_hub("line");
        assert!(!frame(&mut l, 120, 40).contains(link_words::RECONNECTING.text));
        lose(&mut l, &mut s, &hub);
        let key = format!("Ctrl-X {}", link_words::DISCONNECT.text);
        for (w, h) in [(80u16, 24u16), (120, 40)] {
            let f = frame(&mut l, w, h);
            assert!(f.contains(link_words::RECONNECTING.text), "{w}x{h}:\n{f}");
            assert!(f.contains(&key), "{w}x{h}:\n{f}");
        }
        // Settings covers the rest of the shell; a tool covers the pane.
        l.handle_event(&mut s, ShellEvent::OpenSettings);
        let f = frame(&mut l, 80, 24);
        assert!(f.contains(link_words::RECONNECTING.text), "{f}");
        assert!(f.contains(&key), "{f}");
        l.handle_event(&mut s, ShellEvent::CloseSettings);
        l.handle_event(&mut s, ShellEvent::OpenTool(Tool::Matrix));
        assert!(frame(&mut l, 80, 24).contains(&key));
    }

    #[test]
    fn wp_tm8_the_mirror_stays_readable_while_reconnecting() {
        let (mut l, mut s, hub) = on_hub("mirror");
        l.handle_event(&mut s, ShellEvent::Select(Selection::Input(0)));
        turn(&mut l, &mut s);
        let curves = l.shell.model.curves.len();
        assert!(curves > 0);
        let inputs = l.shell.model.inputs.len();
        lose(&mut l, &mut s, &hub);
        for _ in 0..5 {
            turn(&mut l, &mut s);
        }
        assert!(!l.state.connected);
        let m = &l.shell.model;
        assert_eq!(m.selection, Selection::Input(0), "the page stays open");
        assert_eq!(m.inputs.len(), inputs, "the channels stay");
        assert_eq!(m.curves.len(), curves, "the curves stay");
        assert!(
            m.inputs.iter().all(|c| c.level == 0.0),
            "no live-looking meter"
        );
        let f = frame(&mut l, 120, 40);
        assert!(f.contains("INPUTS") && f.contains("Not connected"), "{f}");
        // The overview's grid is the mirror too.
        l.handle_event(&mut s, ShellEvent::Select(Selection::Overview));
        turn(&mut l, &mut s);
        let f = frame(&mut l, 120, 40);
        let name = l.state.channel_name(0);
        assert!(f.contains(&name), "{name}: {f}");
    }

    #[test]
    fn wp_tm8_the_key_stops_the_retries_over_any_overlay() {
        let (mut l, mut s, hub) = on_hub("abort");
        lose(&mut l, &mut s, &hub);
        l.prompt = Some(Prompt::new(false, &l.ctx));
        l.handle_key(&mut s, disconnect_key());
        assert!(l.reconnect.is_none(), "nothing is retrying");
        assert!(!l.shell.model.link_lost);
        assert!(!frame(&mut l, 120, 40).contains(link_words::RECONNECTING.text));
        assert!(l.shell.model.echo.contains(link_words::CLOSED.text));
        // No held mirror now: the Console's view with no device.
        assert_eq!(l.shell.model.channel_count(), 0);

        // The hub takes connections again, and nobody calls.
        hub.refuse_handshake(None);
        l.prompt = None;
        for _ in 0..50 {
            turn(&mut l, &mut s);
        }
        assert!(!l.state.connected);
        assert!(hub.sessions().is_empty(), "{:?}", hub.sessions());

        // And the way back is the person's.
        l.run_command(&mut s, "reconnect");
        turn_until(&mut l, &mut s, "back on request", |l| l.state.connected);
        assert_eq!(hub.sessions().len(), 1);
    }

    /// Abort during a dial: the key does not wait for it, and what it finds is
    /// not adopted.
    #[test]
    fn wp_tm8_the_key_does_not_wait_for_a_dial_in_progress() {
        let (mut l, mut s, hub) = on_hub("dial");
        lose(&mut l, &mut s, &hub);
        // The hub takes the socket and does not answer: the next attempt hangs.
        hub.hold_replies();
        hub.refuse_handshake(None);
        let dialling = |l: &Live| l.reconnect.as_ref().is_some_and(Reconnector::dialling);
        // A refused attempt may still be ending: only a dial that lasts is hung.
        let deadline = Instant::now() + LONG;
        let mut steady = 0;
        while steady < 30 {
            assert!(Instant::now() < deadline, "no dial hung");
            turn(&mut l, &mut s);
            steady = if dialling(&l) { steady + 1 } else { 0 };
        }
        let at = Instant::now();
        l.handle_key(&mut s, disconnect_key());
        assert!(
            at.elapsed() < Duration::from_millis(100),
            "{:?}",
            at.elapsed()
        );
        assert!(l.reconnect.is_none());
        hub.release_replies();
        for _ in 0..200 {
            turn(&mut l, &mut s);
            assert!(!l.state.connected, "a late dial was adopted");
        }
        assert!(hub.sessions().is_empty(), "{:?}", hub.sessions());
    }

    #[test]
    fn wp_tm8_a_write_fails_fast_and_is_never_replayed() {
        let (mut l, mut s, hub) = on_hub("write");
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -3".into()));
        let sent = writes(&hub);
        assert!(sent > 0, "the command went to the hub");
        // A second value inside the interval is held, as a held key's is.
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -4".into()));
        assert_eq!(l.gate.held.len(), 1);
        let image = hub.image();
        let mirror = l.state.bulk.clone();

        lose(&mut l, &mut s, &hub);
        assert!(l.gate.held.is_empty(), "the held write went with the link");
        let said = l.shell.model.status.clone().unwrap_or_default();
        assert!(said.contains("unsent"), "{said:?}");

        // The link is down: at once, an error on screen, nothing changed.
        l.shell.model.status = None;
        let at = Instant::now();
        l.run_command(&mut s, "pre 1 -9");
        assert!(
            at.elapsed() < Duration::from_millis(100),
            "{:?}",
            at.elapsed()
        );
        let said = l.shell.model.status.clone().unwrap_or_default();
        assert!(said.contains("disconnected"), "{said:?}");
        assert!(l.state.bulk == mirror, "the mirror did not move");

        // And the hub comes back to a device nobody touched.
        hub.refuse_handshake(None);
        turn_until(&mut l, &mut s, "the reconnect", |l| l.state.connected);
        for _ in 0..20 {
            turn(&mut l, &mut s);
        }
        assert_eq!(writes(&hub), sent, "a write was replayed");
        assert_eq!(hub.image(), image, "the device changed");
        assert!(l.state.bulk == mirror, "and it reads back the same");
        assert!(!l.shell.model.link_lost);
        assert!(!l.state.mirror_held);
    }

    /// USB and the demo device: the device has gone, so the Console's rule
    /// stands, with no status line and no key.
    #[test]
    fn wp_tm8_a_device_that_left_keeps_the_no_device_view() {
        let c = Target::Demo.connect(&connect::no_prompt).unwrap();
        let mut session = c.session;
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut l = Live::new(
            c.state,
            theme,
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        l.set_connection(&Target::Demo, &c.ext);
        l.state.connected = false;
        l.tick(&mut session, None);
        assert!(!l.shell.model.link_lost);
        assert_eq!(l.shell.model.channel_count(), 0);
        assert!(!frame(&mut l, 120, 40).contains(link_words::RECONNECTING.text));
        l.handle_key(&mut session, disconnect_key());
        assert!(l.reconnect.is_some(), "the key is not this state's");
    }

    /// A bootloader handoff over a hub is the device leaving on purpose.
    #[test]
    fn wp_tm8_a_handoff_to_the_bootloader_is_no_lost_link() {
        let (mut l, mut s, _hub) = on_hub("boot");
        l.boot = Some(actions::BootloaderWatch::new());
        l.state.connected = false;
        l.sync_model();
        assert!(!l.shell.model.link_lost);
        l.handle_key(&mut s, disconnect_key());
        assert!(l.reconnect.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Placeholder;
    use crate::theme::{ColorDepth, Glyphs};
    use crate::widgets::testing::key;
    use dspi_proto::generated::opcodes as op;
    use dspi_session::Capabilities;
    use dspi_transport::mock::LogHandle;
    use dspi_transport::{MockTransport, Transport};

    /// A stand-in screen factory for the runner's tests: every region is a
    /// titled placeholder, so what a test asserts is the runner and not a
    /// screen, and nothing reads the user's config file.
    struct StandInScreens;

    impl Screens for StandInScreens {
        fn detail(&self, state: &DeviceState, selection: Selection) -> Box<dyn Screen> {
            let title = match selection {
                Selection::Overview => "Overview".to_string(),
                Selection::Input(i) => state.channel_name(i),
                Selection::Output(o) => state.channel_name(state.caps.num_inputs as usize + o),
            };
            Box::new(Placeholder::new(title, ""))
        }

        fn tool(&self, _state: &DeviceState, tool: Tool) -> Box<dyn Screen> {
            Box::new(Placeholder::new(tool.title(), ""))
        }

        fn settings(&self, _state: &DeviceState) -> Box<dyn Screen> {
            Box::new(Placeholder::new("Settings", ""))
        }
    }

    fn packet() -> Vec<u8> {
        crate::shell::fixture::packet()
    }

    fn caps() -> Capabilities {
        crate::shell::fixture::caps()
    }

    fn live() -> (Live, Session, LogHandle) {
        let mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            // The readback after a nudge from -12.0 dB must agree with the write.
            .data(op::REQ_GET_USER_VOLUME, (-12.5f32).to_le_bytes().to_vec())
            .data(op::REQ_GET_STATUS, vec![0; 41]);
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps()).unwrap();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        // Tests run without the reveal and the easing, as --lite does, so a
        // value asserted right after a change is the value itself.
        let live = Live::new(state, theme, Performance::lite(), Box::new(StandInScreens));
        (live, session, log)
    }

    #[test]
    fn the_model_is_projected_from_the_device_state() {
        let (l, _, _) = live();
        let m = &l.shell.model;
        assert_eq!(m.inputs.len(), 8);
        assert_eq!(m.outputs.len(), 9);
        assert_eq!(m.inputs[0].name, "FL");
        assert_eq!(m.inputs[1].name, "FR");
        assert_eq!(m.outputs[8].descriptor, "OUT9");
        // The names have not been read back yet, so the slot reads as Empty.
        assert_eq!(m.preset_label, "3: Empty");
        assert!(!m.preset_dirty);
        assert_eq!(m.serial_short, "1B8B4E3A");
        assert!(m.curves.is_empty(), "the overview graphs nothing");
        assert_eq!(m.graph_channel, None);
        // An unnamed channel shows its descriptor instead of a blank.
        let (mut l, _, _) = live();
        let (_, n, _) = dspi_proto::generated::SECTIONS[9];
        l.state.bulk.patch(n + 32, &[0; 32]);
        l.sync_model();
        assert_eq!(l.shell.model.inputs[1].name, "IN2");
    }

    #[test]
    fn the_graph_carries_the_selected_channel_and_its_linked_partner() {
        let (mut l, mut session, _) = live();
        l.select(Selection::Input(0));
        let m = &l.shell.model;
        assert_eq!(m.curves.len(), 1, "unlinked: the channel alone");
        assert_eq!(m.graph_channel.as_deref(), Some("FL"));
        assert!(m.curves[0].selected);
        assert_eq!(m.curves[0].color, l.shell.theme.inputs[0]);

        l.shared.borrow_mut().set_linked(0, true);
        l.sync_model();
        let m = &l.shell.model;
        assert_eq!(m.curves.len(), 2, "linked: the partner underneath");
        assert_eq!(m.curves[0].descriptor, "IN2");
        assert_eq!(m.curves[0].color, l.shell.theme.dim);
        assert!(!m.curves[0].selected);
        assert_eq!(m.curves[1].descriptor, "IN1");

        l.handle_event(&mut session, ShellEvent::GraphPartner);
        assert_eq!(l.shell.model.curves.len(), 1, "`.` hides the partner");
        l.handle_event(&mut session, ShellEvent::GraphPartner);
        assert_eq!(l.shell.model.curves.len(), 2);

        // An output has no partner, whatever the link says.
        l.select(Selection::Output(0));
        assert_eq!(l.shell.model.curves.len(), 1);
        assert_eq!(l.shell.model.graph_channel.as_deref(), Some("OUT L"));
    }

    /// The same runner, wired to the Console's screens rather than the
    /// placeholders.
    fn console() -> (Live, Session, LogHandle) {
        console_with(false)
    }

    /// The same runner against a device that answers every opcode rather than
    /// stalling the ones the fixture does not script.
    ///
    /// The File and Tools verbs touch dozens of opcodes each, and an
    /// unscripted one costs the transport's whole busy-retry backoff; a test
    /// of what reaches the wire should not spend minutes waiting for a mock to
    /// decide it is not going to answer.
    fn answering() -> (Live, Session, LogHandle) {
        console_with(true)
    }

    fn console_with(answer_everything: bool) -> (Live, Session, LogHandle) {
        let mut mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            .data(op::REQ_GET_USER_VOLUME, (-12.5f32).to_le_bytes().to_vec())
            .data(op::REQ_GET_STATUS, vec![0; 41]);
        if answer_everything {
            mock = mock.answering_everything(vec![0; 64]);
        }
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps()).unwrap();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let live = Live::new(
            state,
            theme,
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        (live, session, log)
    }

    #[test]
    fn the_console_screens_are_the_default_detail_region() {
        let (mut l, mut s, _) = console();
        // The overview is what the shell opens on.
        assert_eq!(l.shell.detail.title(), "Overview");
        l.handle_event(&mut s, ShellEvent::Select(Selection::Input(0)));
        assert!(
            l.shell.detail.keys().iter().any(|k| k.does == "Enable All"),
            "the input page carries the filter list's actions"
        );
        l.handle_event(&mut s, ShellEvent::Select(Selection::Output(0)));
        assert!(l.shell.detail.keys().iter().any(|k| k.key == "x"));
    }

    /// `S` and `D` open the beta4 tools by their Console titles and say what
    /// the device lacks; with the feature, `S` is the real panel and `D`'s
    /// placeholder says its panel is still to come.
    #[test]
    fn the_beta4_tool_keys_open_their_tools_or_say_what_is_missing() {
        let (mut l, mut s, _) = console();
        let frame = |l: &mut Live, w, h| crate::render_frame(w, h, |a, b| l.draw(a, b));
        for (tool, title, missing) in [
            (
                Tool::Subharm,
                "Subharmonic Synthesizer",
                "Requires firmware with wire format V29",
            ),
            (
                Tool::Tube,
                "Tube Modeller",
                "Requires firmware with wire format V31",
            ),
        ] {
            l.handle_event(&mut s, ShellEvent::OpenTool(tool));
            assert!(matches!(l.shell.tool, Some((t, _)) if t == tool));
            for (w, h) in [(120u16, 40u16), (80, 24)] {
                let f = frame(&mut l, w, h);
                assert!(f.contains(title), "{w}x{h}:\n{f}");
                assert!(f.contains(&format!("{} closes", tool.key())), "{f}");
                assert!(f.contains(missing), "{w}x{h}:\n{f}");
            }
            l.handle_event(&mut s, ShellEvent::CloseTool);
        }
        for (tool, name, body) in [
            (Tool::Subharm, "subharmonic_synth", "LEVELS"),
            (Tool::Tube, "tube_preamp", "Preamp triodes"),
        ] {
            l.state.caps.features.push(dspi_session::probe::Feature {
                name: name.into(),
                present: true,
                evidence: "test".into(),
            });
            l.handle_event(&mut s, ShellEvent::OpenTool(tool));
            let f = frame(&mut l, 120, 40);
            assert!(f.contains(body), "{f}");
            assert!(!f.contains("Requires firmware"), "{f}");
        }
    }

    /// The Console's no-device state: no channel rows, a graph grid with no
    /// curves, and a channel page falls back to the overview.
    #[test]
    fn losing_the_device_empties_the_sidebar_and_the_graph() {
        let (mut l, mut s, _) = console();
        l.handle_event(&mut s, ShellEvent::Select(Selection::Input(0)));
        assert!(!l.shell.model.curves.is_empty());
        let frame = |l: &mut Live, w, h| crate::render_frame(w, h, |a, b| l.draw(a, b));
        assert!(frame(&mut l, 120, 40).contains("INPUTS"));

        l.state.connected = false;
        l.tick(&mut s, None);
        let m = &l.shell.model;
        assert_eq!(m.selection, Selection::Overview, "back to the overview");
        assert_eq!(m.channel_count(), 0, "no channel rows");
        assert!(
            m.curves.is_empty() && m.graph_channel.is_none(),
            "no curves"
        );
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame(&mut l, w, h);
            assert!(!f.contains("INPUTS") && !f.contains("OUTPUTS"), "{f}");
            assert!(
                f.contains("Not connected") || f.contains("No Devices"),
                "{f}"
            );
            // The graph keeps its grid and axes; the overview has no cells.
            assert!(f.contains("Filter Response"), "{w}x{h}:\n{f}");
            assert!(f.contains("1k"), "the frequency axis: {w}x{h}:\n{f}");
            assert!(!f.contains("╭ FL"), "no overview cells: {f}");
        }
        // With no rows, the sidebar's keys have nothing to select.
        l.handle_event(&mut s, ShellEvent::Select(Selection::Overview));
        assert_eq!(l.shell.model.selection, Selection::Overview);
    }

    #[test]
    fn the_preset_row_opens_the_consoles_whole_menu() {
        let (mut l, mut s, _) = console();
        l.shell.focus = crate::shell::Focus::Footer(crate::shell::FooterRow::Preset);
        l.handle_key(&mut s, key(KeyCode::Enter));
        let (_, popup) = l.popup.as_ref().expect("the preset popup");
        assert_eq!(popup.items[0], "1: Empty");
        assert_eq!(popup.items[11], "Save");
        // Nothing is stored on the fixture device, so the Clear rows are
        // absent, as the Console hides them; Copy to... is the last action.
        assert_eq!(popup.items.last().map(String::as_str), Some("Copy to..."));
        assert!(!popup.items.iter().any(|i| i.starts_with("Clear")));
        // With a stored slot the menu ends in Clear All Slots..., and picking
        // it opens its own step.
        l.popup = None;
        l.shared.borrow_mut().occupied = 0b101;
        l.handle_key(&mut s, key(KeyCode::Enter));
        let (_, popup) = l.popup.as_ref().expect("the preset popup");
        assert_eq!(
            popup.items.last().map(String::as_str),
            Some("Clear All Slots...")
        );
        l.handle_key(&mut s, key(KeyCode::End));
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(matches!(l.dialog, Some((AppDialog::PresetClearAll, _))));
    }

    #[test]
    fn copy_and_paste_carry_a_channels_parameters() {
        let (mut l, mut s, log) = console();
        l.handle_event(&mut s, ShellEvent::CopyParams(0));
        assert!(l.shared.borrow().clipboard.is_some());
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Copied FL parameters")
        );
        l.handle_event(&mut s, ShellEvent::PasteParams(1));
        let writes = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .count();
        assert_eq!(writes, 10, "every band travels");
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|s| s.starts_with("Pasted FL onto")),
            "{:?}",
            l.shell.model.status
        );
    }

    #[test]
    fn nothing_to_paste_says_so() {
        let (mut l, mut s, _) = console();
        l.handle_event(&mut s, ShellEvent::PasteParams(1));
        assert_eq!(l.shell.model.status.as_deref(), Some("Nothing to paste"));
    }

    /// A screen asks for several writes at once when one gesture is several
    /// writes: a mirrored edit, a Clear All, a paste.
    #[test]
    fn a_multi_line_command_runs_every_line() {
        let (mut l, mut s, log) = console();
        l.handle_event(
            &mut s,
            ShellEvent::Command("eq.bypass in.1 2 on\neq.bypass in.2 2 on".into()),
        );
        let writes = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_BAND_BYPASS)
            .count();
        assert_eq!(writes, 2);
        assert_eq!(l.shell.model.echo, "eq.bypass in.2 2 on");
    }

    #[test]
    fn a_volume_nudge_writes_the_user_volume() {
        let (mut l, mut s, log) = live();
        l.shell.focus = crate::shell::Focus::Footer(crate::shell::FooterRow::Volume);
        l.handle_key(&mut s, key(KeyCode::Left));
        let wrote = log
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.opcode == op::REQ_SET_USER_VOLUME);
        assert!(wrote, "expected a SET_USER_VOLUME on the wire");
        assert!(
            l.shell.model.echo.contains("vol.user"),
            "{}",
            l.shell.model.echo
        );
    }

    #[test]
    fn the_command_line_runs_the_shared_grammar() {
        let (mut l, mut s, log) = live();
        l.handle_key(&mut s, key(KeyCode::Char(':')));
        assert!(l.prompt.is_some());
        for c in "bypass on".chars() {
            l.handle_key(&mut s, key(KeyCode::Char(c)));
        }
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.prompt.is_none());
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_SET_BYPASS)
        );
    }

    #[test]
    fn quitting_with_unsaved_changes_asks_first() {
        let (mut l, mut s, _) = live();
        let (_, g, _) = dspi_proto::generated::SECTIONS[1];
        l.state.bulk.patch(g, &(-6.0f32).to_le_bytes());
        l.sync_model();
        assert!(l.shell.model.preset_dirty);
        l.handle_key(&mut s, key(KeyCode::Char('q')));
        assert!(!l.should_quit);
        assert!(matches!(l.dialog, Some((AppDialog::Unsaved { .. }, _))));
        l.handle_key(&mut s, key(KeyCode::Char('d')));
        assert!(l.should_quit, "Discard quits");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = connect::test_dir().join(format!("dspi-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        dir.join(name)
    }

    /// Every File and Tools verb is offered by the palette, or half the
    /// program is unreachable without reading the source.
    #[test]
    fn the_palette_offers_the_interfaces_own_verbs() {
        let (l, _, _) = console();
        let p = Prompt::new(true, &l.ctx);
        for (verb, _) in APP_VERBS {
            assert!(
                p.candidates.iter().any(|c| c.value == *verb),
                "{verb} is not in the palette"
            );
        }
        // And they complete like anything else.
        let mut p = Prompt::new(true, &l.ctx);
        p.input = "import".into();
        p.refresh(&l.ctx);
        let offered: Vec<&str> = p.candidates.iter().map(|c| c.value.as_str()).collect();
        assert!(offered.contains(&"import") && offered.contains(&"import-config"));
    }

    // -- WP UTM -- (UXP A13)
    fn palette(l: &mut Live, s: &mut Session, query: &str) {
        l.handle_key(s, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(l.prompt.as_ref().is_some_and(|p| p.palette));
        for c in query.chars() {
            l.handle_key(s, key(KeyCode::Char(c)));
        }
    }

    /// "crossover" in the palette, Enter: an output's page on its XO tab.
    #[test]
    fn utm_the_palette_opens_the_crossover() {
        let (mut l, mut s, _) = console();
        palette(&mut l, &mut s, "crossover");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.prompt.is_none());
        assert!(matches!(l.shell.model.selection, Selection::Output(_)));
        assert_eq!(l.shell.focus, crate::shell::Focus::Screen);
        let tabs = l.shell.detail.actions(&l.state);
        assert!(
            tabs.contains(&("XO".to_string(), false)),
            "the XO tab is the one showing: {tabs:?}"
        );
    }

    /// "mute" in the palette, Enter: the listening volume's mute.
    #[test]
    fn utm_the_palette_mutes() {
        let (mut l, mut s, log) = live();
        palette(&mut l, &mut s, "mute");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_SET_USER_MUTE),
            "a mute on the wire"
        );
    }

    /// Nothing found: the palette says so and stays open on Enter.
    #[test]
    fn utm_the_palette_says_when_nothing_matches() {
        let (mut l, mut s, _) = live();
        palette(&mut l, &mut s, "zzzz");
        let p = l.prompt.as_ref().unwrap();
        assert_eq!(p.candidates.len(), 1);
        assert!(p.candidates[0].detail.starts_with("No match for"), "{p:?}");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.prompt.is_some(), "Enter on nothing keeps the palette");
    }

    /// A whole command typed into the palette still runs.
    #[test]
    fn utm_a_command_typed_into_the_palette_still_runs() {
        let (mut l, mut s, log) = live();
        palette(&mut l, &mut s, "bypass on");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_SET_BYPASS)
        );
    }

    /// A parameter goes to the `:` line for its value.
    #[test]
    fn utm_a_parameter_found_goes_to_the_command_line() {
        let (mut l, mut s, _) = live();
        palette(&mut l, &mut s, "vol.mute");
        l.handle_key(&mut s, key(KeyCode::Enter));
        let p = l.prompt.as_ref().expect("the : line");
        assert!(!p.palette);
        assert_eq!(p.input, "vol.mute ");
    }

    /// `:help` is `?`.
    #[test]
    fn utm_help_on_the_command_line_opens_the_keys() {
        let (mut l, mut s, _) = live();
        l.run_command(&mut s, "help");
        assert!(l.shell.help);
    }

    /// UXP C10: the shell's verbs run on this device, through the embedder,
    /// instead of "Try `uconsole params`".
    #[test]
    fn utm_the_shell_verbs_run_inside_the_interface() {
        let (mut l, mut s, _) = live();
        l.run_command(&mut s, "snapshot list");
        assert!(l.shell_run.is_none());
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("`snapshot` only works from the shell"),
            "no embedder, no verb"
        );
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = std::rc::Rc::clone(&ran);
        l.set_shell_verbs(std::rc::Rc::new(move |w: &[String], _: Option<&Target>| {
            seen.borrow_mut().extend(w.iter().cloned());
            0
        }));
        l.run_command(&mut s, "snapshot list");
        let words = l.shell_run.take().expect("handed to the run loop");
        assert_eq!(words, ["snapshot", "list"]);
        assert_eq!(l.run_shell_verb(&mut s, &words), 0);
        assert_eq!(*ran.borrow(), ["snapshot", "list"]);
        assert_eq!(l.shell.model.status.as_deref(), Some("Ran :snapshot list"));
        // Measuring stays a one-shot; the documents run here.
        l.run_command(&mut s, "measure run");
        assert!(l.shell_run.is_none());
        l.run_command(&mut s, "measure docs");
        assert!(l.shell_run.is_some());
    }

    /// UXP C10: an unknown word points inside, not at the shell.
    #[test]
    fn utm_an_unknown_word_points_to_search_and_help() {
        let (mut l, mut s, _) = live();
        l.run_command(&mut s, "speakers");
        let said = l.shell.model.status.clone().unwrap_or_default();
        assert!(said.contains("Ctrl-P searches"), "{said}");
        assert!(!said.contains("params"), "{said}");
    }

    /// UXP A13: `1k` on the `:` line.
    #[test]
    fn utm_the_command_line_takes_k() {
        let (mut l, mut s, log) = live();
        l.run_command(&mut s, "eq in.1 1 peak 1k 1.4 -3");
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_SET_EQ_PARAM),
            "{:?}",
            l.shell.model.status
        );
    }

    #[test]
    fn export_writes_a_filter_file_without_asking_when_the_path_came_with_the_verb() {
        let (mut l, mut s, _) = console();
        let path = scratch("tuning.txt");
        l.run_command(&mut s, &format!("export {}", path.display()));
        assert!(l.dialog.is_none(), "no dialog when the path was given");
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Filters exported successfully")
        );
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.contains("# DSPi Console Filter Settings"), "{text}");
        assert!(text.contains("[Input 0: FL]"), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// The whole import flow: a path, then the channel checklist, then the
    /// bands on the wire and the Console's report.
    #[test]
    fn import_asks_which_channels_then_writes_the_bands() {
        let (mut l, mut s, log) = answering();
        let path = scratch("rew.txt");
        std::fs::write(
            &path,
            "Preamp: -6.5 dB\nFilter 1: ON PK Fc 100 Hz Gain 3.0 dB Q 1.00\n",
        )
        .unwrap();

        l.run_command(&mut s, &format!("import {}", path.display()));
        let (kind, dialog) = l.dialog.as_ref().expect("the channel picker");
        assert!(matches!(kind, AppDialog::ImportChannels { .. }));
        assert_eq!(dialog.title, "Import Filters");
        assert!(
            dialog
                .body
                .contains("Found 1 filter(s) and a -6.5 dB preamp")
        );

        // Take everything but the first channel off, then import.
        l.handle_key(&mut s, key(KeyCode::Down));
        for _ in 0..7 {
            l.handle_key(&mut s, key(KeyCode::Char(' ')));
            l.handle_key(&mut s, key(KeyCode::Down));
        }
        l.handle_key(&mut s, key(KeyCode::Enter));

        let bands = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .count();
        assert_eq!(bands, 10, "one channel's whole bank, filled and cleared");
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Filters imported to 1 channel(s)")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_config_export_and_import_round_trip_through_the_options_checklist() {
        let (mut l, mut s, _) = answering();
        let path = scratch("config.dspipreset");
        l.run_command(&mut s, &format!("export-config {}", path.display()));
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|m| m.starts_with("Configuration exported.")),
            "{:?}",
            l.shell.model.status
        );

        l.run_command(&mut s, &format!("import-config {}", path.display()));
        let (kind, dialog) = l.dialog.as_ref().expect("the options checklist");
        assert!(matches!(kind, AppDialog::ConfigOptions(_)));
        assert!(
            dialog.body.contains(
                "EQ, crossover, delays, gains, routing and the DSP features are always applied."
            ),
            "{}",
            dialog.body
        );
        // Import with both options left off, which is the Console's default.
        l.handle_key(&mut s, key(KeyCode::Enter));
        let (kind, dialog) = l.dialog.as_ref().expect("the report");
        assert!(matches!(kind, AppDialog::Report));
        match &dialog.kind {
            crate::widgets::DialogKind::Report { lines, .. } => {
                assert!(lines[0].starts_with("Applied "), "{lines:?}");
                assert!(
                    lines
                        .last()
                        .unwrap()
                        .contains("These changes are live but not yet stored on the device."),
                    "{lines:?}"
                );
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_path_dialog_completes_on_tab() {
        let (mut l, mut s, _) = console();
        let path = scratch("completable.txt");
        std::fs::write(&path, "x").unwrap();
        l.run_command(&mut s, "import");
        let stem = path
            .to_string_lossy()
            .replace("completable.txt", "completab");
        if let Some((_, d)) = l.dialog.as_mut()
            && let crate::widgets::DialogKind::Text { value, .. } = &mut d.kind
        {
            *value = stem;
        }
        l.handle_key(&mut s, key(KeyCode::Tab));
        let Some((_, d)) = l.dialog.as_ref() else {
            panic!("the dialog closed")
        };
        match &d.kind {
            crate::widgets::DialogKind::Text { value, .. } => {
                assert_eq!(value, &path.to_string_lossy().to_string())
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_tools_verbs_confirm_before_they_act() {
        let (mut l, mut s, log) = answering();

        l.run_command(&mut s, "commit");
        assert_eq!(
            l.dialog.as_ref().unwrap().1.body,
            "Save current parameters to preset slot 3?"
        );
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_PRESET_SAVE),
            "Save writes the preset"
        );

        l.run_command(&mut s, "factory-reset");
        let (_, d) = l.dialog.as_ref().unwrap();
        assert!(d.critical, "a factory reset is a critical confirm");
        // Escape leaves the device alone.
        l.handle_key(&mut s, key(KeyCode::Esc));
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_FACTORY_RESET),
            "cancelling wrote nothing"
        );
        l.run_command(&mut s, "factory-reset");
        l.handle_key(&mut s, key(KeyCode::Char('r')));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_FACTORY_RESET)
        );
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Factory reset complete")
        );
    }

    /// Revert re-loads the active slot: the old synchronous load opcode was
    /// reassigned because it crashed the device on S/PDIF input.
    #[test]
    fn revert_reloads_the_active_preset_rather_than_the_reassigned_opcode() {
        let (mut l, mut s, log) = console();
        l.run_command(&mut s, "revert");
        assert_eq!(l.dialog.as_ref().unwrap().1.title, "Revert to Saved");
        l.handle_key(&mut s, key(KeyCode::Char('r')));
        let sent = log.lock().unwrap();
        assert!(sent.iter().any(|e| e.opcode == op::REQ_PRESET_LOAD));
        assert!(
            !sent.iter().any(|e| e.opcode == op::REQ_SAVE_OUTPUT_CONFIG),
            "0x52 is Save Output Configuration now, not Load Params"
        );
    }

    // -- the beta4 sections through the write path ---------------------------

    fn section_at(name: &str) -> usize {
        dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, o, _)| *o)
            .unwrap()
    }

    /// A runner whose device already holds `after`: the state starts from the
    /// fixture packet, so whatever the re-read after a write brings is what
    /// the device changed on its own.
    fn beta4(after: Vec<u8>, mock: MockTransport) -> (Live, Session, LogHandle) {
        let mut caps = caps();
        for name in ["subharmonic_synth", "tube_preamp", "output_limiter"] {
            caps.features.push(dspi_session::probe::Feature {
                name: name.into(),
                present: true,
                evidence: "test".into(),
            });
        }
        let mock = mock
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, after)
            .data(op::REQ_GET_STATUS, vec![0; 41])
            .answering_everything(vec![0; 64]);
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps.clone()).unwrap();
        let state = DeviceState::new(
            caps,
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let live = Live::new(state, theme, Performance::lite(), Box::new(StandInScreens));
        (live, session, log)
    }

    /// Choosing a tube type makes the device load that type's bias,
    /// asymmetry, hardness and sag (tube.c:122-212). The Terminal does not
    /// copy the row itself; the re-read that follows every write brings the
    /// four values in (DESIGN section 11).
    #[test]
    fn a_tube_type_write_brings_its_four_values_back_with_the_reread() {
        let row = dspi_session::tube::type_row(3).unwrap();
        let mut after = packet();
        let t = section_at("tube");
        after[t + 1] = 3;
        for (i, v) in [row.bias_pct, row.asym_db, row.hardness_pct, row.sag_pct]
            .iter()
            .enumerate()
        {
            after[t + 12 + 4 * i..t + 16 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        let (mut l, mut s, log) = beta4(
            after,
            MockTransport::new().data(op::REQ_GET_TUBE_PARAM, 3.0f32.to_le_bytes().to_vec()),
        );
        assert_eq!(l.state.tube().tube_type, 0);
        l.run_command(&mut s, "tube.type 3");
        let tube = l.state.tube();
        assert_eq!(tube.tube_type, 3);
        assert_eq!(
            (tube.bias_pct, tube.asym_db, tube.hardness_pct, tube.sag_pct),
            (5.0, 2.0, 55.0, 10.0),
            "the 12AT7 row"
        );
        let sent = log.lock().unwrap();
        let set = sent
            .iter()
            .position(|e| e.opcode == op::REQ_SET_TUBE_PARAM)
            .unwrap();
        assert!(
            sent[set..]
                .iter()
                .any(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK),
            "a whole-packet re-read follows the one write"
        );
    }

    /// A write to one member of a limiter link group moves the whole group
    /// (limiter.c:143-194); the re-read shows the others moving too, with no
    /// copy of the ganging rules here.
    #[test]
    fn a_limiter_write_on_a_linked_output_brings_the_group_back() {
        let l0 = section_at("limiter");
        let mut after = packet();
        for o in 0..2 {
            after[l0 + 12 * o + 1] = 1;
            after[l0 + 12 * o + 4..l0 + 12 * o + 8].copy_from_slice(&(-6.0f32).to_le_bytes());
        }
        let (mut l, mut s, _) = beta4(
            after,
            MockTransport::new().data(op::REQ_LIMITER, (-6.0f32).to_le_bytes().to_vec()),
        );
        l.run_command(&mut s, "limit.threshold 0 -6");
        assert_eq!(l.state.limiter(0).unwrap().threshold_db, -6.0);
        assert_eq!(l.state.limiter(1).unwrap().threshold_db, -6.0);
        assert_eq!(l.state.limiter(1).unwrap().link_group, 1);
        assert_eq!(l.state.limiter(2).unwrap().threshold_db, 0.0);
        // WITH_PRESET, the default: a preset change, not output config.
        assert!(l.state.has_unsaved_changes());
        assert!(!l.state.limiter_unsaved());
    }

    /// In INDEPENDENT mode the same write is output configuration: the
    /// preset stays clean, Settings shows it unsaved, and Save Output
    /// Configuration takes it as saved.
    #[test]
    fn in_independent_mode_a_limiter_write_waits_for_save_output_configuration() {
        let l0 = section_at("limiter");
        let mut after = packet();
        after[l0 + 4..l0 + 8].copy_from_slice(&(-6.0f32).to_le_bytes());
        let mut dir = dspi_proto::packets::PresetDirectory::default().encode();
        dir[5] = 0; // output_config_mode INDEPENDENT (config.h:497)
        let (mut l, mut s, _) = beta4(
            after,
            MockTransport::new()
                .data(op::REQ_LIMITER, (-6.0f32).to_le_bytes().to_vec())
                .data(op::REQ_PRESET_GET_DIR, dir.to_vec()),
        );
        l.refresh_presets(&mut s);
        assert_eq!(l.state.output_config_mode, 0, "read with the directory");
        l.run_command(&mut s, "limit.threshold 0 -6");
        assert!(!l.state.has_unsaved_changes(), "the preset is untouched");
        assert!(l.state.limiter_unsaved());
        l.run_command(&mut s, "save-output-config");
        assert!(!l.state.limiter_unsaved());

        // Undo past the save moves the limiters off the saved configuration,
        // so the save bar has to come back. The mock's bulk read does not
        // follow writes, so the device's side is patched in by hand.
        l.handle_event(&mut s, ShellEvent::Undo);
        assert!(l.state.limiter_baseline.is_some(), "the saved baseline");
        l.state
            .bulk
            .patch(section_at("limiter") + 4, &0.0f32.to_le_bytes());
        assert!(l.state.limiter_unsaved());
        assert!(!l.state.has_unsaved_changes(), "the preset is untouched");
    }

    /// The master-volume mode comes with the directory and follows a write,
    /// so the unsaved check counts the master volume only with the preset.
    #[test]
    fn the_master_volume_mode_is_read_with_the_directory_and_follows_writes() {
        let mut dir = dspi_proto::packets::PresetDirectory::default().encode();
        dir[6] = 1; // master_volume_mode WITH_PRESET (config.h:485)
        let (mut l, mut s, _) = beta4(
            packet(),
            MockTransport::new().data(op::REQ_PRESET_GET_DIR, dir.to_vec()),
        );
        assert_eq!(l.state.master_volume_mode, 0, "INDEPENDENT until read");
        l.refresh_presets(&mut s);
        assert_eq!(l.state.master_volume_mode, 1, "read with the directory");
        l.state
            .bulk
            .patch(section_at("master_volume"), &(-30.0f32).to_le_bytes());
        assert!(l.state.has_unsaved_changes());
        // A flash write, confirmed as the Settings dialog would.
        l.run_command(&mut s, "vol.master.mode independent");
        let (kind, _) = l.dialog.take().unwrap();
        l.finish_dialog(&mut s, kind, DialogOutcome::Button(0));
        assert_eq!(l.state.master_volume_mode, 0, "follows the write");
        assert!(!l.state.has_unsaved_changes());
    }

    /// The gain-reduction meter is read on the meter tick only while an
    /// output page is showing and some limiter is on, as the Console polls
    /// it only while its icon is on screen; otherwise the reading is cleared.
    #[test]
    fn the_limiter_meter_is_polled_only_on_an_output_page_with_a_limiter_on() {
        let l0 = section_at("limiter");
        let mut meter = vec![0u8; 18];
        meter[0..2].copy_from_slice(&320u16.to_le_bytes());
        let (mut l, mut s, log) =
            beta4(packet(), MockTransport::new().data(op::REQ_LIMITER, meter));
        let reads = |log: &LogHandle| {
            log.lock()
                .unwrap()
                .iter()
                .filter(|e| {
                    e.opcode == op::REQ_LIMITER
                        && e.value == dspi_proto::generated::limiter::LIMITER_GET_METER
                })
                .count()
        };
        l.state.bulk.patch(l0, &[1]);
        l.tick(&mut s, None);
        assert_eq!(reads(&log), 0, "the overview does not show the limiter");
        l.shell.model.selection = Selection::Output(0);
        l.state.bulk.patch(l0, &[0]);
        l.tick(&mut s, None);
        assert_eq!(reads(&log), 0, "every limiter is off");
        l.state.bulk.patch(l0, &[1]);
        l.tick(&mut s, None);
        l.tick(&mut s, None);
        assert_eq!(reads(&log), 2, "once per meter tick");
        assert_eq!(screens::limiter::reduction_db(&l.state, 0), 3.2);
        l.shell.model.selection = Selection::Overview;
        l.tick(&mut s, None);
        assert_eq!(reads(&log), 2);
        assert!(l.state.limiter_meter.is_none(), "cleared once, not polled");
    }

    /// Space on the output page's limiter cell switches output 1 on; the
    /// device switches its linked partner too (limiter.c:143-194), and the
    /// re-read after the write is what shows it.
    #[test]
    fn a_limiter_switched_on_from_the_output_page_brings_its_linked_partner_on() {
        use crate::shell::Screen;
        let l0 = section_at("limiter");
        let mut after = packet();
        for o in 0..2 {
            after[l0 + 12 * o] = 1;
            after[l0 + 12 * o + 1] = 1;
        }
        let (mut l, mut s, _) = beta4(
            after,
            MockTransport::new().data(op::REQ_LIMITER, 1.0f32.to_le_bytes().to_vec()),
        );
        for o in 0..2 {
            l.state.bulk.patch(l0 + 12 * o + 1, &[1]);
        }
        let mut page = screens::OutputPage::new(0, screens::shared(), &l.state);
        for _ in 0..4 {
            page.handle(key(KeyCode::Down), &l.state);
        }
        page.handle(key(KeyCode::Right), &l.state);
        let crate::shell::ScreenEvent::Command(c) = page.handle(key(KeyCode::Char(' ')), &l.state)
        else {
            panic!("Space on the limiter cell writes");
        };
        assert_eq!(c, "limit.on 0 on");
        assert!(!l.state.limiter(1).unwrap().enabled);
        l.run_command(&mut s, &c);
        assert!(l.state.limiter(0).unwrap().enabled);
        assert!(
            l.state.limiter(1).unwrap().enabled,
            "the partner came back with the re-read"
        );
        assert!(!l.state.limiter(2).unwrap().enabled);
    }

    /// In INDEPENDENT mode a limiter edit from the output page's command bar
    /// leaves the preset clean and puts the save bar up in Settings.
    #[test]
    fn in_independent_mode_the_save_bar_shows_a_limiter_edit() {
        use crate::shell::Screen;
        let l0 = section_at("limiter");
        let mut after = packet();
        after[l0 + 4..l0 + 8].copy_from_slice(&(-6.0f32).to_le_bytes());
        let mut dir = dspi_proto::packets::PresetDirectory::default().encode();
        dir[5] = 0; // output_config_mode INDEPENDENT (config.h:497)
        let (mut l, mut s, _) = beta4(
            after,
            MockTransport::new()
                .data(op::REQ_LIMITER, (-6.0f32).to_le_bytes().to_vec())
                .data(op::REQ_PRESET_GET_DIR, dir.to_vec()),
        );
        l.refresh_presets(&mut s);
        let page = screens::OutputPage::new(0, screens::shared(), &l.state);
        let q = page.quick("limit -6", &l.state).unwrap();
        assert_eq!(q.commands, vec!["limit.threshold 0 -6"]);
        let mut settings = SettingsScreen::new(
            &l.state,
            crate::settings::tests::data(),
            AppConfig::default(),
        )
        .open(crate::settings::Page::About, &l.state);
        let before = crate::settings::tests::frame(&mut settings, &l.state, 120, 40);
        assert!(
            !before.contains(crate::widgets::SaveBar::FLASH),
            "nothing unsaved yet:\n{before}"
        );
        l.run_command(&mut s, &q.commands[0]);
        assert!(!l.state.has_unsaved_changes(), "the preset is untouched");
        let f = crate::settings::tests::frame(&mut settings, &l.state, 120, 40);
        assert!(f.contains(crate::widgets::SaveBar::FLASH), "{f}");
    }

    /// While the Subharmonic Synthesizer panel is open the runner reads its
    /// headroom and solo, reads the headroom again only when the section
    /// changes, meters only while the module is on, and switches solo off
    /// once when the panel closes, as the Console's window does.
    #[test]
    fn the_subharm_panel_polls_its_runtime_state_and_clears_solo_on_close() {
        let (mut l, mut s, log) = beta4(
            packet(),
            MockTransport::new()
                .data(op::REQ_GET_SUBHARM_SOLO, vec![1])
                .data(op::REQ_GET_SUBHARM_HEADROOM, 4.5f32.to_le_bytes().to_vec()),
        );
        let count = |log: &LogHandle, opcode: u8| {
            log.lock()
                .unwrap()
                .iter()
                .filter(|e| e.opcode == opcode)
                .count()
        };
        l.tick(&mut s, None);
        assert_eq!(count(&log, op::REQ_GET_SUBHARM_SOLO), 0, "not while closed");

        l.handle_event(&mut s, ShellEvent::OpenTool(Tool::Subharm));
        l.tick(&mut s, None);
        assert_eq!(l.state.subharm_solo, Some(true));
        assert_eq!(l.state.subharm_headroom_db, Some(4.5));
        assert_eq!(count(&log, op::REQ_GET_SUBHARM_HEADROOM), 1);
        assert_eq!(
            count(&log, op::REQ_GET_SUBHARM_METER),
            0,
            "the module is off, so nothing to meter"
        );
        l.tick(&mut s, None);
        assert_eq!(
            count(&log, op::REQ_GET_SUBHARM_HEADROOM),
            1,
            "the section has not changed"
        );
        assert_eq!(count(&log, op::REQ_GET_SUBHARM_SOLO), 1, "once a second");

        // Switching the module on moves the section: headroom and meters.
        let o = section_at("subharm");
        l.state.bulk.patch(o, &[1]);
        l.tick(&mut s, None);
        assert_eq!(count(&log, op::REQ_GET_SUBHARM_HEADROOM), 2);
        assert_eq!(count(&log, op::REQ_GET_SUBHARM_METER), 1);

        l.handle_event(&mut s, ShellEvent::CloseTool);
        l.tick(&mut s, None);
        let offs: Vec<Vec<u8>> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_SUBHARM_SOLO)
            .map(|e| e.payload.clone())
            .collect();
        assert_eq!(offs, vec![vec![0]], "solo off, once");
        l.tick(&mut s, None);
        assert_eq!(count(&log, op::REQ_SET_SUBHARM_SOLO), 1, "and only once");
    }

    /// The palette offers the panel, and `subharm` opens it.
    #[test]
    fn the_palette_finds_the_subharmonic_synthesizer() {
        let (mut l, mut s, _) = console();
        let mut p = Prompt::new(true, &l.ctx);
        p.input = "subh".into();
        p.refresh(&l.ctx);
        assert!(
            p.candidates
                .iter()
                .any(|c| c.value == "subharm" && c.detail == "Subharmonic Synthesizer")
        );
        l.run_command(&mut s, "subharm");
        assert!(matches!(l.shell.tool, Some((Tool::Subharm, _))));
    }

    /// A change this build cannot place in the shadow is not dropped: the
    /// runner re-reads the whole packet, as the Console resyncs on an offset
    /// it does not decode (survey-console-beta4 section 3).
    #[test]
    fn a_change_past_the_packet_makes_the_runner_reread() {
        let (mut l, mut s, log) = beta4(packet(), MockTransport::new());
        let notes = MockTransport::new();
        let mut p = vec![2, 2, 0, 1];
        p.extend((dspi_proto::generated::BULK_SIZE as u16).to_le_bytes());
        p.extend(4u16.to_le_bytes());
        p.extend([5, 0, 0, 0]);
        p.extend(1.0f32.to_le_bytes());
        notes.push_notification(p);
        let n = Notifications::start(notes.notifications().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        log.lock().unwrap().clear();
        l.tick(&mut s, Some(&n));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK),
            "the whole packet is read again"
        );
        assert!(!l.state.stale);
    }

    #[test]
    fn save_master_and_save_output_config_report_what_they_did() {
        let (mut l, mut s, log) = answering();
        l.run_command(&mut s, "save-master");
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|m| m.starts_with("Master volume saved (")),
            "{:?}",
            l.shell.model.status
        );
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|m| m.ends_with("It will be applied on next boot.")),
            "{:?}",
            l.shell.model.status
        );
        l.run_command(&mut s, "save-output-config");
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Output configuration saved. It will be applied on next boot.")
        );
        let sent = log.lock().unwrap();
        assert!(sent.iter().any(|e| e.opcode == op::REQ_SAVE_MASTER_VOLUME));
        assert!(sent.iter().any(|e| e.opcode == op::REQ_SAVE_OUTPUT_CONFIG));
    }

    /// The bootloader jump has no acknowledgement and every later transfer
    /// fails, so what the person sees afterwards is the whole of the feature.
    #[test]
    fn the_firmware_update_confirms_then_waits_for_the_drive() {
        let (mut l, mut s, log) = console();
        l.run_command(&mut s, "bootloader");
        let (_, d) = l.dialog.as_ref().expect("the confirm");
        assert!(d.critical);
        assert_eq!(d.buttons[0].label, "Reboot into Bootloader");
        l.handle_key(&mut s, key(KeyCode::Char('r')));

        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_ENTER_BOOTLOADER)
        );
        assert!(l.boot.is_some(), "the wait started");
        let (kind, d) = l.dialog.as_ref().expect("the progress dialog");
        assert!(matches!(kind, AppDialog::BootWait));
        assert_eq!(d.title, "Reboot into Bootloader");
        match &d.kind {
            crate::widgets::DialogKind::Progress { status, .. } => {
                assert_eq!(status, "Waiting for the device to disconnect...")
            }
            other => panic!("{other:?}"),
        }
    }

    /// The Tube Modeller is reachable from the palette by name, as well as by
    /// its `D` key.
    #[test]
    fn the_tube_verb_opens_the_tube_modeller() {
        let (mut l, mut s, _) = console();
        l.run_command(&mut s, "tube");
        assert!(matches!(l.shell.tool, Some((Tool::Tube, _))));
    }

    #[test]
    fn the_autoeq_verb_opens_the_browser_and_its_update_menu() {
        let (mut l, mut s, _) = console();
        l.run_command(&mut s, "autoeq");
        assert!(matches!(l.shell.tool, Some((Tool::AutoEq, _))));

        l.run_command(&mut s, "autoeq update");
        let (kind, d) = l.dialog.as_ref().expect("the update menu");
        assert!(matches!(kind, AppDialog::AutoEqUpdate(_)));
        assert_eq!(d.title, "Update AutoEQ Database");
        assert!(d.body.contains("Choose an update method:"), "{}", d.body);
        assert_eq!(d.buttons[0].label, "Rebuild from GitHub");
        assert_eq!(d.buttons[1].label, "Import File...");

        // The rebuild is opt-in behind a warning that names what it fetches.
        l.handle_key(&mut s, key(KeyCode::Enter));
        let (kind, d) = l.dialog.as_ref().expect("the rebuild confirm");
        assert!(matches!(kind, AppDialog::AutoEqRebuild));
        assert!(d.body.contains("api.github.com"), "{}", d.body);
        assert_eq!(d.default, 1, "Cancel is the default");
    }

    /// The Update Database buttons are matched by what they stand for, so
    /// Cancel in the third place without "Reset to Built-in" never resets.
    #[test]
    fn the_update_menu_matches_its_buttons_by_method() {
        let (mut l, mut s, _) = console();
        let methods = vec![AutoEqMethod::Rebuild, AutoEqMethod::Import];
        l.finish_dialog(
            &mut s,
            AppDialog::AutoEqUpdate(methods.clone()),
            DialogOutcome::Button(2),
        );
        assert!(l.dialog.is_none(), "the third button is Cancel here");

        l.finish_dialog(
            &mut s,
            AppDialog::AutoEqUpdate(methods),
            DialogOutcome::Button(0),
        );
        let (kind, _) = l.dialog.as_ref().expect("the rebuild confirm");
        assert!(matches!(kind, AppDialog::AutoEqRebuild));
    }

    /// One device is not a picker; the Console only offers one with more than
    /// one attached.
    #[test]
    fn the_device_picker_says_so_when_there_is_nothing_to_pick() {
        let (mut l, mut s, _) = console();
        l.handle_event(&mut s, ShellEvent::DevicePicker);
        assert!(l.dialog.is_none());
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .is_some_and(|m| m.contains("device")),
            "{:?}",
            l.shell.model.status
        );
    }

    /// A switch with unsaved changes asks first, and only goes through once
    /// the answer is in.
    #[test]
    fn switching_device_with_unsaved_changes_asks_first() {
        let (mut l, mut s, _) = console();
        let (_, g, _) = dspi_proto::generated::SECTIONS[1];
        l.state.bulk.patch(g, &(-6.0f32).to_le_bytes());
        l.sync_model();
        l.switch_device("OTHER0000000001".into());
        assert!(l.switch_to.is_none(), "not yet");
        assert!(matches!(l.dialog, Some((AppDialog::Unsaved { .. }, _))));
        l.handle_key(&mut s, key(KeyCode::Char('d')));
        assert_eq!(l.switch_to.as_deref(), Some("OTHER0000000001"));
    }

    #[test]
    fn a_notification_from_elsewhere_lands_in_the_model_and_the_echo_line() {
        let (mut l, mut s, _) = live();
        let mock = MockTransport::new();
        let (_, u, _) = dspi_proto::generated::SECTIONS[16];
        let mut p = vec![2, 2, 0, 1];
        p.extend((u as u16).to_le_bytes());
        p.extend(4u16.to_le_bytes());
        p.extend([7, 0, 0, 0]);
        p.extend((-30.0f32).to_le_bytes());
        mock.push_notification(p);
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        l.tick(&mut s, Some(&n));
        assert_eq!(l.state.user_volume().0, -30.0);
        assert_eq!(l.shell.model.volume_db, -30.0);
        assert_eq!(
            l.shell.model.echo,
            "user volume changed by the system volume"
        );
    }

    /// An ADAT state change brings the Stats poll forward, so the panel's
    /// ADAT section follows the link without waiting out the two seconds.
    #[test]
    fn an_adat_state_change_brings_the_stats_poll_forward() {
        let (mut l, mut s, _) = live();
        l.open_tool(Tool::Stats);
        let polled = Instant::now();
        l.last_stats_poll = polled;
        let mock = MockTransport::new();
        mock.push_notification(vec![2, 8, 0, 13, 1, 1, 12, 0]);
        let n = Notifications::start(mock.notifications().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        l.tick(&mut s, Some(&n));
        assert_eq!(l.state.adat_state, Some((true, true, 12)));
        assert!(
            l.last_stats_poll < polled,
            "the next tick reads Stats again"
        );
    }

    /// The analyser is read inside the loop's own tick, after the meters,
    /// only while its panel watches; closing the panel stops the device, and
    /// a waiting key makes it sit a tick out.
    #[test]
    fn the_analyser_is_polled_while_its_panel_is_open_and_stopped_after() {
        use crate::screens::spectrum::demo;
        use dspi_transport::mock::{Direction, Reply};
        let centres = |r: std::ops::Range<usize>| -> Vec<u8> {
            demo::CENTRES[r]
                .iter()
                .flat_map(|c| c.to_le_bytes())
                .collect()
        };
        let mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            .data(op::REQ_GET_STATUS, vec![0; 41])
            .reply(
                op::REQ_RTA_GET_CAPS,
                Reply::Sequence(vec![
                    Reply::Data(demo::caps().encode().to_vec()),
                    Reply::Data(centres(0..32)),
                    Reply::Data(centres(32..37)),
                ]),
            )
            .data(op::REQ_RTA_GET_BANDS, demo::frame(0).encode().to_vec())
            .data(op::REQ_RTA_CONTROL, vec![1]);
        let log = mock.log_handle();
        let mut s = Session::new(Box::new(mock), caps()).unwrap();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut l = Live::new(
            state,
            theme,
            Performance::lite(),
            Box::new(ConsoleScreens::new()),
        );
        l.connect_analyser(&mut s);
        assert!(l.shared.borrow().spectrum.engine.supported());
        let rta = |o: u8| (op::REQ_RTA_SET_CONFIG..=op::REQ_RTA_GET_BANDS_ALL).contains(&o);

        // Nothing watches yet, so nothing is read.
        log.lock().unwrap().clear();
        l.tick(&mut s, None);
        assert!(!log.lock().unwrap().iter().any(|x| rta(x.opcode)));

        l.handle_event(&mut s, ShellEvent::OpenTool(Tool::Spectrum));
        log.lock().unwrap().clear();
        l.tick(&mut s, None);
        let ops: Vec<u8> = log.lock().unwrap().iter().map(|x| x.opcode).collect();
        let first_rta = ops.iter().position(|o| rta(*o)).expect("the analyser ran");
        assert!(
            first_rta > 0 && !rta(ops[0]),
            "the meters go first: {ops:02X?}"
        );
        assert!(ops.contains(&op::REQ_RTA_GET_BANDS), "{ops:02X?}");

        // A key waiting: the analyser sits this tick out.
        l.input_pending = true;
        log.lock().unwrap().clear();
        l.tick(&mut s, None);
        assert!(!log.lock().unwrap().iter().any(|x| rta(x.opcode)));
        l.input_pending = false;

        l.handle_event(&mut s, ShellEvent::CloseTool);
        log.lock().unwrap().clear();
        l.tick(&mut s, None);
        let stops: Vec<(Direction, u16)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|x| x.opcode == op::REQ_RTA_CONTROL)
            .map(|x| (x.direction, x.value))
            .collect();
        assert_eq!(stops, vec![(Direction::In, 0)], "STOP, wValue 0");
    }

    /// A change Settings > Graphing saves reaches the graph, the overview's
    /// layout and the cursor readout without a restart.
    #[test]
    fn a_saved_graphing_change_reaches_the_graph() {
        let (_, mut s, _) = console();
        let screens = ConsoleScreens::new();
        let config = screens.config.clone();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut l = Live::new(state, theme, Performance::lite(), Box::new(screens));
        {
            let mut c = config.borrow_mut();
            c.graphing.grid = crate::graph::GridStrength::Off;
            c.graphing.dashboard_cards = 2;
            c.graphing.freq_readout = false;
            c.graphing.gain_readout = true;
        }
        l.adopt_graphing();
        assert_eq!(l.shell.model.graph.grid, crate::graph::GridStrength::Off);
        assert_eq!(l.shared.borrow().graph.dashboard_cards, 2);
        l.handle_event(&mut s, ShellEvent::Select(Selection::Input(0)));
        l.handle_event(&mut s, ShellEvent::GraphCursor(Some(1000.0)));
        let status = l.shell.model.status.clone().unwrap_or_default();
        assert!(!status.contains("Hz"), "no frequency: {status}");
        config.borrow_mut().graphing.gain_readout = false;
        l.adopt_graphing();
        l.shell.model.status = None;
        l.handle_event(&mut s, ShellEvent::GraphCursor(Some(500.0)));
        assert_eq!(l.shell.model.status, None, "nothing to read out");
        assert_eq!(
            l.shell.model.cursor_hz,
            Some(500.0),
            "the cursor still moves"
        );
    }

    /// The writes a held arrow key makes, counted against the mock at a
    /// typical 30 Hz key repeat and at a fast 60 Hz one, one second each.
    #[test]
    fn a_held_key_writes_at_most_thirty_times_a_second_and_always_the_last_value() {
        use dspi_transport::mock::Direction;
        let sets = |log: &LogHandle| -> Vec<f32> {
            log.lock()
                .unwrap()
                .iter()
                .filter(|x| x.direction == Direction::Out && x.opcode == op::REQ_SET_PREAMP_CH)
                .map(|x| f32::from_le_bytes(x.payload[..4].try_into().unwrap()))
                .collect()
        };
        for (hz, most) in [(30u32, 30usize), (60, 31)] {
            let (mut l, mut s, log) = answering();
            log.lock().unwrap().clear();
            let t0 = Instant::now();
            let period = Duration::from_secs(1) / hz;
            let mut last = 0.0;
            for i in 0..hz {
                l.clock = Some(t0 + period * i);
                // A screen that keeps its own running value, as the band
                // editor does, so every repeat asks for a new one.
                last = -20.0 + i as f32 * 0.1;
                l.handle_event(
                    &mut s,
                    ShellEvent::Command(format!("pre 1 {}", screens::number(last))),
                );
                l.flush_writes(&mut s, false);
            }
            let during = sets(&log).len();
            assert!(during <= most, "{hz} Hz: {during} writes in a second");
            // The key is let go: the held value goes once its interval ends.
            l.clock = Some(t0 + period * hz + WRITE_INTERVAL);
            l.flush_writes(&mut s, false);
            let all = sets(&log);
            assert_eq!(
                *all.last().unwrap(),
                last,
                "{hz} Hz: the final value is sent"
            );
            assert!(all.len() <= most + 1, "{hz} Hz: {} writes", all.len());
            if hz == 30 {
                assert_eq!(all.len(), 30, "one write per repeat at 30 Hz");
            }
        }
    }

    /// Toggles are not held back, and a second parameter is its own value.
    #[test]
    fn toggles_and_other_values_are_not_held_back() {
        use dspi_transport::mock::Direction;
        let (mut l, mut s, log) = answering();
        l.clock = Some(Instant::now());
        log.lock().unwrap().clear();
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -3".into()));
        l.handle_event(&mut s, ShellEvent::Command("pre 2 -3".into()));
        l.handle_event(&mut s, ShellEvent::Command("bypass on".into()));
        l.handle_event(&mut s, ShellEvent::Command("bypass off".into()));
        let ops: Vec<u8> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|x| x.direction == Direction::Out)
            .map(|x| x.opcode)
            .collect();
        assert_eq!(
            ops.iter().filter(|o| **o == op::REQ_SET_PREAMP_CH).count(),
            2
        );
        assert_eq!(ops.iter().filter(|o| **o == op::REQ_SET_BYPASS).count(), 2);
        // The same value again inside the interval is held, and undo sees it
        // land first.
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -4".into()));
        assert_eq!(l.gate.held.len(), 1);
        l.flush_writes(&mut s, true);
        assert!(l.gate.held.is_empty());
    }

    /// `live()` on a device that answers `REQ_CLEAR_CLIPS` when `clears`,
    /// and refuses it (as a hub does for a viewer) otherwise.
    fn live_clips(clears: bool) -> (Live, Session, LogHandle) {
        let mut mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            .data(op::REQ_GET_STATUS, vec![0; 41]);
        if clears {
            mock = mock.data(op::REQ_CLEAR_CLIPS, vec![0; 4]);
        }
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps()).unwrap();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let live = Live::new(state, theme, Performance::lite(), Box::new(StandInScreens));
        (live, session, log)
    }

    fn clip_meters(peak: f32, clip: bool) -> dspi_session::Meters {
        dspi_session::Meters {
            peaks: vec![peak; 17],
            clipped: vec![clip; 1],
            cpu0: 0,
            cpu1: 0,
            active_inputs: 2,
        }
    }

    fn clears_sent(log: &LogHandle) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|x| x.opcode == op::REQ_CLEAR_CLIPS)
            .count()
    }

    /// WP P0M: the meters' hold and clip hold are the shared ballistics: the
    /// clip marker stays ten seconds after the last new clip (the Console's),
    /// then the device is asked to forget. Review p0m F1: a channel that keeps
    /// clipping is shown again on the next read, as the old Terminal and the
    /// Console re-latched.
    #[test]
    fn wp_p0m_the_clip_hold_is_the_shared_one() {
        let (mut l, mut session, log) = live_clips(true);
        let t0 = Instant::now();
        let s = |secs: u64, ms: u64| t0 + Duration::from_secs(secs) + Duration::from_millis(ms);
        l.on_meters(clip_meters(0.7, true), t0, &mut session);
        assert!(l.state.is_clipped(0));
        assert_eq!(l.meters.hold(0), 0.7);
        // Three seconds on: the old Terminal cleared here; the hold stays.
        l.on_meters(clip_meters(0.1, true), s(3, 0), &mut session);
        assert!(l.state.is_clipped(0));
        assert_eq!(clears_sent(&log), 0);
        // Ten seconds: cleared, dark for this read.
        l.on_meters(clip_meters(0.1, true), s(10, 0), &mut session);
        assert_eq!(clears_sent(&log), 1);
        assert!(!l.state.is_clipped(0));
        // Still clipping: the next read shows it again.
        l.on_meters(clip_meters(0.1, true), s(10, 60), &mut session);
        assert!(l.state.is_clipped(0));
        // Over two minutes of clipping, dark for one read per clear at most.
        let (mut dark, mut t) = (0, 10_120u64);
        while t <= 130_000 {
            l.on_meters(
                clip_meters(1.0, true),
                t0 + Duration::from_millis(t),
                &mut session,
            );
            dark += usize::from(!l.state.is_clipped(0));
            t += 60;
        }
        assert!(clears_sent(&log) >= 12);
        assert!(dark < clears_sent(&log), "dark {dark} reads");
        // A user's clear darkens it at once; the device then reads clear.
        l.handle_event(&mut session, ShellEvent::ClearClips);
        assert!(!l.state.is_clipped(0));
        l.on_meters(clip_meters(0.1, false), s(131, 0), &mut session);
        assert!(!l.state.is_clipped(0));
        assert_eq!(l.meters.clip_hold(), 0);
    }

    /// Review p0m D1: where the clear is refused (a viewer on a hub), the
    /// marker follows the device's sticky bit instead of going dark.
    #[test]
    fn wp_p0m_a_refused_clear_keeps_the_clip_shown() {
        let (mut l, mut session, log) = live_clips(false);
        let t0 = Instant::now();
        l.on_meters(clip_meters(0.7, true), t0, &mut session);
        let mut t = 500u64;
        while t <= 60_000 {
            l.on_meters(
                clip_meters(0.1, true),
                t0 + Duration::from_millis(t),
                &mut session,
            );
            assert!(l.state.is_clipped(0), "shown at {t} ms");
            t += 500;
        }
        assert_eq!(clears_sent(&log), 1, "asked once, never retried");
        // Cleared elsewhere: dark ten seconds after the last read with the bit.
        l.on_meters(
            clip_meters(0.1, false),
            t0 + Duration::from_millis(60_500),
            &mut session,
        );
        assert!(l.state.is_clipped(0));
        l.on_meters(
            clip_meters(0.1, false),
            t0 + Duration::from_millis(70_100),
            &mut session,
        );
        assert!(!l.state.is_clipped(0));
    }

    /// The writes that reach the wire, by opcode, in order.
    fn sent_ops(log: &LogHandle) -> Vec<u8> {
        use dspi_transport::mock::Direction;
        log.lock()
            .unwrap()
            .iter()
            .filter(|x| x.direction == Direction::Out)
            .map(|x| x.opcode)
            .collect()
    }

    /// A held arrow on a bypassed band: the band's `eq` line sends it active
    /// (vendor_commands.c:545-550), so its bypass flag is held with it and
    /// goes after it, never ahead of it.
    #[test]
    fn a_held_bypassed_band_keeps_its_bypass_after_the_band() {
        let (mut l, mut s, log) = answering();
        let t0 = Instant::now();
        l.clock = Some(t0);
        log.lock().unwrap().clear();
        let band = |g: &str| format!("eq in.1 1 peak 1000 1 {g}\neq.bypass in.1 1 on");
        l.handle_event(&mut s, ShellEvent::Command(band("3")));
        l.clock = Some(t0 + Duration::from_millis(10));
        l.handle_event(&mut s, ShellEvent::Command(band("3.5")));
        assert_eq!(l.gate.held.len(), 1, "the band and its flag, held as one");
        l.flush_writes(&mut s, false);
        l.clock = Some(t0 + WRITE_INTERVAL + Duration::from_millis(1));
        l.flush_writes(&mut s, false);
        let ops: Vec<u8> = sent_ops(&log)
            .into_iter()
            .filter(|o| [op::REQ_SET_EQ_PARAM, op::REQ_SET_BAND_BYPASS].contains(o))
            .collect();
        assert_eq!(
            ops,
            vec![
                op::REQ_SET_EQ_PARAM,
                op::REQ_SET_BAND_BYPASS,
                op::REQ_SET_EQ_PARAM,
                op::REQ_SET_BAND_BYPASS
            ],
            "the flag always follows its band"
        );
    }

    /// A write that is never held (a toggle, a reset, a menu action) sends
    /// the held ones first, so it cannot overtake them.
    #[test]
    fn a_write_that_is_not_held_goes_after_the_held_ones() {
        let (mut l, mut s, log) = answering();
        l.clock = Some(Instant::now());
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -3".into()));
        log.lock().unwrap().clear();
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -4\nbypass on".into()));
        assert!(l.gate.held.is_empty());
        let ops = sent_ops(&log);
        let pre = ops.iter().position(|o| *o == op::REQ_SET_PREAMP_CH);
        let byp = ops.iter().position(|o| *o == op::REQ_SET_BYPASS);
        assert!(pre.is_some() && pre < byp, "{ops:02X?}");

        // The same from the shell: the volume reset is not a screen's line.
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -5".into()));
        assert_eq!(l.gate.held.len(), 1);
        log.lock().unwrap().clear();
        l.handle_event(&mut s, ShellEvent::VolumeReset);
        let ops = sent_ops(&log);
        let pre = ops.iter().position(|o| *o == op::REQ_SET_PREAMP_CH);
        let vol = ops.iter().position(|o| *o == op::REQ_SET_USER_VOLUME);
        assert!(pre.is_some() && pre < vol, "{ops:02X?}");
    }

    /// A held write is sent before the Terminal lets go of the device, and is
    /// never carried over to the next one.
    #[test]
    fn held_writes_go_before_letting_go_and_not_to_the_next_device() {
        let (mut l, mut s, log) = answering();
        l.clock = Some(Instant::now());
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -3".into()));
        l.handle_event(&mut s, ShellEvent::Command("pre 1 -4".into()));
        log.lock().unwrap().clear();
        l.release(&mut s);
        assert!(sent_ops(&log).contains(&op::REQ_SET_PREAMP_CH));
        assert!(l.gate.held.is_empty());

        l.handle_event(&mut s, ShellEvent::Command("pre 1 -5".into()));
        assert_eq!(l.gate.held.len(), 1);
        let state = l.state.clone();
        l.adopt(&mut s, state);
        assert!(
            l.gate.held.is_empty(),
            "the held write was the old device's"
        );
    }

    // -- the deferred preset actions ------------------------------------------

    /// `[version, event, flags, seq, byte]`: a PRESET_LOADED or a
    /// BULK_INVALIDATED (notify.h:37-41, 96-97).
    fn event(id: u8, seq: u8, byte: u8) -> Vec<u8> {
        vec![2, id, 0, seq, byte]
    }

    const PRESET_LOADED: u8 = 0x04;
    const BULK_INVALIDATED: u8 = 0x03;
    const FROM_PRESET: u8 = 3;
    const FROM_FACTORY: u8 = 4;

    /// A runner whose device answers the active slot in turn from `active`,
    /// and whose notification reader holds `notes`.
    fn presets_rig(active: &[u8], notes: &[Vec<u8>]) -> (Live, Session, LogHandle) {
        use dspi_transport::mock::Reply;
        let mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            .data(op::REQ_GET_STATUS, vec![0; 41])
            .data(op::REQ_PRESET_LOAD, vec![0])
            .reply(
                op::REQ_PRESET_GET_ACTIVE,
                Reply::Sequence(active.iter().map(|a| Reply::Data(vec![*a])).collect()),
            )
            .answering_everything(vec![0; 64]);
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps()).unwrap();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut live = Live::new(state, theme, Performance::lite(), Box::new(StandInScreens));
        let reader = MockTransport::new();
        for n in notes {
            reader.push_notification(n.clone());
        }
        live.notes = Some(Notifications::start(reader.notifications().unwrap()).unwrap());
        (live, session, log)
    }

    fn dirty(l: &mut Live) {
        let (_, g, _) = dspi_proto::generated::SECTIONS[1];
        l.state.bulk.patch(g, &(-6.0f32).to_le_bytes());
        l.sync_model();
        assert!(l.state.has_unsaved_changes());
    }

    /// Copy To with Discard reloads the source first. The load is deferred
    /// behind a fade, so the saves wait for it to finish, and the second
    /// save waits for the first to be taken: the device keeps one pending
    /// save slot (vendor_commands.c:2679-2694).
    #[test]
    fn copy_to_after_discard_waits_for_the_load_and_for_each_save() {
        use dspi_transport::mock::Direction;
        // The fixture's active slot is 2; the copy goes to 5.
        let (mut l, mut s, log) = presets_rig(
            &[2, 2, 2, 2, 5, 5, 2],
            &[
                event(PRESET_LOADED, 1, 2),
                event(BULK_INVALIDATED, 2, FROM_PRESET),
            ],
        );
        dirty(&mut l);
        l.copy_preset_to(&mut s, 5);
        assert!(matches!(l.dialog, Some((AppDialog::Unsaved { .. }, _))));
        l.handle_key(&mut s, key(KeyCode::Char('d')));

        let sent: Vec<(u8, u16)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::In)
            .map(|e| (e.opcode, e.value))
            .collect();
        let at = |opcode: u8, value: u16| {
            sent.iter()
                .position(|x| *x == (opcode, value))
                .unwrap_or_else(|| panic!("{opcode:02X} {value} not sent: {sent:02X?}"))
        };
        let load = at(op::REQ_PRESET_LOAD, 2);
        let to_dest = at(op::REQ_PRESET_SAVE, 5);
        let to_source = at(op::REQ_PRESET_SAVE, 2);
        assert!(
            sent[load..to_dest]
                .iter()
                .any(|x| x.0 == op::REQ_GET_ALL_PARAMS_CHUNK),
            "the load is re-read before the first save"
        );
        let confirmed = sent[to_dest..to_source]
            .iter()
            .filter(|x| x.0 == op::REQ_PRESET_GET_ACTIVE)
            .count();
        assert!(confirmed >= 2, "the first save is seen taken: {sent:02X?}");
        assert!(!l.state.has_unsaved_changes());
        assert_eq!(l.state.caps.active_preset, Some(2));
    }

    // -- WP FIN015 --
    /// Copy to… saves the live settings to the destination and then to the
    /// source. With the active slot unknown, or not the one the device names
    /// right before the first save, it saves nothing: slot 0 (or 255) never
    /// stands in for the source (REVF S1, code-safety P1).
    #[test]
    fn copy_to_saves_nothing_from_an_active_slot_it_does_not_know() {
        use dspi_transport::mock::Direction;
        let saves = |log: &LogHandle| -> Vec<u16> {
            log.lock()
                .unwrap()
                .iter()
                .filter(|e| e.direction == Direction::In && e.opcode == op::REQ_PRESET_SAVE)
                .map(|e| e.value)
                .collect()
        };
        // Unknown (a failed read), past the last slot, and a slot the device
        // no longer names (it says 4, the Terminal last saw 2).
        for (shown, device) in [(None, 2u8), (Some(255), 255), (Some(2), 4)] {
            let (mut l, mut s, log) = presets_rig(&[device], &[]);
            l.state.caps.active_preset = shown;
            assert!(!l.state.has_unsaved_changes());
            l.copy_preset_to(&mut s, 5);
            assert!(l.dialog.is_none());
            assert_eq!(saves(&log), Vec::<u16>::new(), "{shown:?}: nothing saved");
            let note = l.shell.model.status.clone().unwrap_or_default();
            assert!(
                note.contains("active"),
                "{shown:?}: the refusal says why: {note}"
            );
        }
        // The Discard of the unsaved-changes question loads no slot 0 either.
        let (mut l, mut s, log) = presets_rig(&[2], &[]);
        l.state.caps.active_preset = Some(2);
        dirty(&mut l);
        l.copy_preset_to(&mut s, 5);
        assert!(matches!(l.dialog, Some((AppDialog::Unsaved { .. }, _))));
        l.state.caps.active_preset = None;
        l.handle_key(&mut s, key(KeyCode::Char('d')));
        let loads = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::In && e.opcode == op::REQ_PRESET_LOAD)
            .count();
        assert_eq!(loads, 0);
        assert_eq!(saves(&log), Vec::<u16>::new());
        // The rule's own case still copies: the device names the slot shown.
        let (mut l, mut s, log) = presets_rig(&[2, 2, 5, 5, 2], &[]);
        l.state.caps.active_preset = Some(2);
        l.copy_preset_to(&mut s, 5);
        assert_eq!(saves(&log), vec![5, 2]);
    }

    /// A slot that fails its CRC check still announces the load, but the
    /// device keeps its active slot (flash_storage.c:4316-4327): that is the
    /// failure, and the baseline stays where it was.
    #[test]
    fn a_load_the_device_did_not_take_is_reported_as_corrupt() {
        let (mut l, mut s, _) = presets_rig(
            &[2],
            &[
                event(PRESET_LOADED, 1, 5),
                event(BULK_INVALIDATED, 2, FROM_PRESET),
            ],
        );
        let before = l.state.saved.clone();
        assert!(!l.load_preset(&mut s, 5));
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Load Failed: Preset data is corrupted.")
        );
        assert_eq!(l.state.caps.active_preset, Some(2));
        assert_eq!(l.state.saved, before);
    }

    /// The factory reset is deferred too: the re-read and the new baseline
    /// wait for its BULK_INVALIDATED (flash_storage.c:4947-4953).
    #[test]
    fn a_factory_reset_waits_for_the_device_before_rebaselining() {
        let (mut l, mut s, _) = presets_rig(&[2], &[]);
        let queue = {
            // A reader whose event arrives only after a while, as the reset's
            // does after the fade.
            let reader = MockTransport::new();
            let q = reader.notify_queue();
            l.notes = Some(Notifications::start(reader.notifications().unwrap()).unwrap());
            q
        };
        let delay = Duration::from_millis(200);
        let pusher = std::thread::spawn(move || {
            std::thread::sleep(delay);
            queue
                .lock()
                .unwrap()
                .push_back(event(BULK_INVALIDATED, 1, FROM_FACTORY));
        });
        dirty(&mut l);
        let start = Instant::now();
        l.factory_reset(&mut s);
        assert!(start.elapsed() >= delay, "returned before the reset ended");
        pusher.join().unwrap();
        assert!(!l.state.has_unsaved_changes());
        assert_eq!(
            l.shell.model.status.as_deref(),
            Some("Factory reset complete")
        );
    }

    /// A preset loaded elsewhere and not yet seen becomes the baseline
    /// before a write of ours goes out, so the write is not counted as part
    /// of the preset.
    #[test]
    fn a_preset_loaded_elsewhere_is_taken_before_our_next_write() {
        use dspi_transport::mock::Direction;
        let (mut l, mut s, log) = presets_rig(
            &[4],
            &[
                event(PRESET_LOADED, 1, 4),
                event(BULK_INVALIDATED, 2, FROM_PRESET),
            ],
        );
        std::thread::sleep(Duration::from_millis(50));
        l.run_command(&mut s, "bypass on");
        let ops: Vec<u8> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::In || e.opcode == op::REQ_SET_BYPASS)
            .map(|e| e.opcode)
            .collect();
        let active = ops.iter().position(|o| *o == op::REQ_PRESET_GET_ACTIVE);
        let write = ops.iter().position(|o| *o == op::REQ_SET_BYPASS);
        assert!(active.is_some() && active < write, "{ops:02X?}");
        assert_eq!(l.state.caps.active_preset, Some(4));
    }

    /// A re-read that fails leaves the shadow marked untrusted, so the next
    /// tick tries again.
    #[test]
    fn a_failed_reread_keeps_the_shadow_stale() {
        let mock = MockTransport::new().data(op::REQ_GET_STATUS, vec![0; 41]);
        let mut s = Session::new(Box::new(mock), caps()).unwrap();
        let (mut l, _, _) = live();
        l.state.stale = true;
        l.tick(&mut s, None);
        assert!(l.state.stale);
    }

    /// The echo line shows what the grammar accepts, so it can be typed back:
    /// output and slot indices are 0-based there.
    #[test]
    fn the_echo_for_outputs_and_slots_is_what_the_grammar_takes() {
        let (mut l, mut s, _) = answering();
        l.run_command(&mut s, "out.enable 2 off");
        assert_eq!(l.shell.model.echo, "out.enable 2 off");
        l.echo_set("preset.save", &[4], Value::Trigger);
        assert_eq!(l.shell.model.echo, "preset.save 4");
        let tokens = dspi_cmd::tokenize(&l.shell.model.echo);
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        assert!(matches!(
            dspi_cmd::parse(&refs, &l.ctx),
            Ok(dspi_cmd::Command::Set { indices, .. }) if indices == vec![4]
        ));
    }

    /// Quitting or switching device with the sub solo on switches it off, as
    /// closing the panel does.
    #[test]
    fn letting_go_of_the_device_switches_the_sub_solo_off() {
        let (mut l, mut s, log) = beta4(
            packet(),
            MockTransport::new().data(op::REQ_GET_SUBHARM_SOLO, vec![0]),
        );
        l.release(&mut s);
        assert!(
            !sent_ops(&log).contains(&op::REQ_SET_SUBHARM_SOLO),
            "solo is not on"
        );
        l.state.subharm_solo = Some(true);
        l.release(&mut s);
        let offs: Vec<Vec<u8>> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_SUBHARM_SOLO)
            .map(|e| e.payload.clone())
            .collect();
        assert_eq!(offs, vec![vec![0]]);
        assert_eq!(l.state.subharm_solo, Some(false));
    }

    // -- WP TM18 --
    /// A runner with its settings file at `path`, on a device that answers
    /// everything and has a signal generator. Every call is a fresh start of
    /// the app as far as the settings go.
    fn tm18_start(path: &std::path::Path) -> (Live, Session, LogHandle) {
        let mut caps = caps();
        caps.features.push(dspi_session::probe::Feature {
            name: "test_signals".into(),
            present: true,
            evidence: "mock".into(),
        });
        caps.siggen = Some(dspi_session::probe::SiggenCaps {
            version: 1,
            type_count: 15,
            output_channels: 9,
            multitone_max: 16,
            valid_channel_mask: 0x1FF,
            types: Vec::new(),
        });
        let mock = MockTransport::new()
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
            .answering_everything(vec![0; 64]);
        let log = mock.log_handle();
        let session = Session::new(Box::new(mock), caps.clone()).unwrap();
        let state = DeviceState::new(
            caps,
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let live = Live::new(
            state,
            Theme::console(ColorDepth::TrueColor, Glyphs::Braille),
            Performance::lite(),
            Box::new(ConsoleScreens::with_config_path(path.to_path_buf())),
        );
        (live, session, log)
    }

    #[test]
    fn tm18_the_volume_mode_is_back_after_a_restart() {
        use crate::settings::config::VolumeChoice;
        let dir = crate::test_dir("tm18-volume");
        let path = dir.join("config.toml");
        let (mut l, mut s, _) = tm18_start(&path);
        assert_eq!(l.shell.model.volume_mode, VolumeMode::User, "the default");
        l.handle_event(&mut s, ShellEvent::VolumeModeToggle);
        assert_eq!(l.shell.model.volume_mode, VolumeMode::Master);
        assert_eq!(
            AppConfig::load_from(&path).volume.mode,
            VolumeChoice::Master
        );
        drop(l);

        let (mut l, mut s, _) = tm18_start(&path);
        assert_eq!(l.shell.model.volume_mode, VolumeMode::Master, "restored");
        l.handle_event(&mut s, ShellEvent::VolumeModeToggle);
        drop(l);
        let (l, _, _) = tm18_start(&path);
        assert_eq!(l.shell.model.volume_mode, VolumeMode::User, "and back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The mode stays changed for this run when the file cannot be written,
    /// and says so.
    #[test]
    fn tm18_a_volume_mode_that_cannot_be_saved_says_so() {
        let dir = crate::test_dir("tm18-volume-blocked");
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("a-file");
        std::fs::write(&blocker, "").unwrap();
        let (mut l, mut s, _) = tm18_start(&blocker.join("config.toml"));
        l.handle_event(&mut s, ShellEvent::VolumeModeToggle);
        l.adopt_volume_choice();
        assert_eq!(l.shell.model.volume_mode, VolumeMode::Master);
        let status = l.shell.model.status.clone().unwrap_or_default();
        assert!(status.contains("Could not save settings"), "{status}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Settings and the sidebar's button hold one choice.
    #[test]
    fn tm18_a_mode_saved_in_settings_reaches_the_sidebar() {
        use crate::settings::config::VolumeChoice;
        let dir = crate::test_dir("tm18-volume-settings");
        let screens = ConsoleScreens::with_config_path(dir.join("config.toml"));
        let config = screens.config.clone();
        let state = DeviceState::new(
            caps(),
            dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
        );
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut l = Live::new(state, theme, Performance::lite(), Box::new(screens));
        config.borrow_mut().volume.mode = VolumeChoice::Master;
        l.adopt_volume_choice();
        assert_eq!(l.shell.model.volume_mode, VolumeMode::Master);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A factory that keeps no settings does not undo the button.
    #[test]
    fn tm18_the_mode_holds_without_a_settings_file() {
        let (mut l, mut s, _) = live();
        l.handle_event(&mut s, ShellEvent::VolumeModeToggle);
        l.adopt_volume_choice();
        assert_eq!(l.shell.model.volume_mode, VolumeMode::Master);
    }

    #[test]
    fn tm18_the_tube_layout_is_back_after_a_restart() {
        use crate::screens::panel::testing;
        use crate::shell::fixture;
        let dir = crate::test_dir("tm18-tube");
        let path = dir.join("config.toml");
        let mut state = fixture::state();
        fixture::tube(&mut state, true);
        let advanced = testing::draw(
            &mut crate::screens::TubePanel::with_view(crate::screens::tube::View::Advanced),
            &state,
            100,
            60,
        );
        let opened = |path: &std::path::Path| {
            ConsoleScreens::with_config_path(path.to_path_buf()).tool(&state, Tool::Tube)
        };
        let mut panel = opened(&path);
        assert_ne!(testing::draw(panel.as_mut(), &state, 100, 60), advanced);
        assert_eq!(
            panel.handle(key(KeyCode::Char('a')), &state),
            crate::shell::ScreenEvent::Handled
        );
        drop(panel);
        let mut panel = opened(&path);
        assert_eq!(testing::draw(panel.as_mut(), &state, 100, 60), advanced);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sidebar's `i` puts the Console's Identify on the wire: the stage
    /// and then the start, for the output the row names.
    #[test]
    fn tm18_identify_reaches_the_wire() {
        let dir = crate::test_dir("tm18-identify");
        let (mut l, mut s, log) = tm18_start(&dir.join("config.toml"));
        let row = l.state.caps.num_inputs as usize + 1;
        l.handle_event(&mut s, ShellEvent::Identify(row));
        let log = log.lock().unwrap();
        let written: Vec<_> = log
            .iter()
            .filter(|e| matches!(e.opcode, op::REQ_SIGGEN_SET_CONFIG | op::REQ_SIGGEN_CONTROL))
            .collect();
        assert_eq!(written.len(), 2, "{written:?}");
        let c = dspi_proto::packets::SiggenConfig::decode(&written[0].payload).unwrap();
        assert_eq!(written[0].opcode, op::REQ_SIGGEN_SET_CONFIG);
        assert_eq!((c.signal_type, c.channel_mask), (14, 0x2));
        assert_eq!((c.level_db, c.repeat_count), (-12.0, 2));
        // A write the firmware takes as a read, with its value in wValue.
        assert_eq!(
            (written[1].opcode, written[1].value),
            (op::REQ_SIGGEN_CONTROL, 1)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Undo and redo through the runner, reading what reaches the wire on a
    /// device whose generator status follows what it was told.
    mod tm18_generator_stays_put {
        use super::*;
        use dspi_transport::{DeviceDescriptor, NotificationSource, Result as TR, TransportCaps};
        use std::sync::atomic::{AtomicU8, Ordering};
        use std::sync::{Arc, Mutex};

        const START: u16 = 1;
        const STOP: u16 = 0;

        struct Generator {
            inner: MockTransport,
            state: Arc<AtomicU8>,
            controls: Arc<Mutex<Vec<u16>>>,
        }

        impl Generator {
            fn status(&self) -> Vec<u8> {
                dspi_proto::packets::SiggenStatus {
                    state: self.state.load(Ordering::SeqCst),
                    signal_type: 14,
                    ..Default::default()
                }
                .encode()
                .to_vec()
            }
        }

        impl Transport for Generator {
            fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TR<Vec<u8>> {
                if opcode == op::REQ_SIGGEN_CONTROL {
                    self.controls.lock().unwrap().push(value);
                    // Running, or fading out from a plain Stop.
                    let state = match value {
                        START => 2,
                        STOP => 4,
                        _ => 0,
                    };
                    self.state.store(state, Ordering::SeqCst);
                    return Ok(vec![1; len as usize]);
                }
                if opcode == op::REQ_SIGGEN_GET_STATUS {
                    let mut b = self.status();
                    b.resize(len as usize, 0);
                    return Ok(b);
                }
                self.inner.control_in(opcode, value, len)
            }
            fn control_in_upto(&mut self, opcode: u8, value: u16, len: u16) -> TR<Vec<u8>> {
                if opcode == op::REQ_SIGGEN_GET_STATUS {
                    return Ok(self.status());
                }
                self.inner.control_in_upto(opcode, value, len)
            }
            fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TR<()> {
                self.inner.control_out(opcode, value, data)
            }
            fn descriptor(&self) -> &DeviceDescriptor {
                self.inner.descriptor()
            }
            fn max_transfer(&self) -> usize {
                self.inner.max_transfer()
            }
            fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
                None
            }
            fn caps(&self) -> TransportCaps {
                self.inner.caps()
            }
        }

        struct Rig {
            live: Live,
            session: Session,
            controls: Arc<Mutex<Vec<u16>>>,
            log: LogHandle,
            dir: std::path::PathBuf,
        }

        impl Rig {
            fn start(name: &str) -> Self {
                let dir = crate::test_dir(name);
                let mut caps = caps();
                caps.features.push(dspi_session::probe::Feature {
                    name: "test_signals".into(),
                    present: true,
                    evidence: "mock".into(),
                });
                caps.siggen = Some(dspi_session::probe::SiggenCaps {
                    version: 1,
                    type_count: 15,
                    output_channels: 9,
                    multitone_max: 16,
                    valid_channel_mask: 0x1FF,
                    types: Vec::new(),
                });
                let mock = MockTransport::new()
                    .window(op::REQ_GET_ALL_PARAMS_CHUNK, packet())
                    .answering_everything(vec![0; 64]);
                let log = mock.log_handle();
                let controls = Arc::new(Mutex::new(Vec::new()));
                let device = Generator {
                    inner: mock,
                    state: Arc::new(AtomicU8::new(0)),
                    controls: controls.clone(),
                };
                let session = Session::new(Box::new(device), caps.clone()).unwrap();
                let state = DeviceState::new(
                    caps,
                    dspi_proto::wire::BulkPacket::decode(packet()).unwrap(),
                );
                let live = Live::new(
                    state,
                    Theme::console(ColorDepth::TrueColor, Glyphs::Braille),
                    Performance::lite(),
                    Box::new(ConsoleScreens::with_config_path(dir.join("config.toml"))),
                );
                Self {
                    live,
                    session,
                    controls,
                    log,
                    dir,
                }
            }
            fn identify(&mut self) {
                let row = self.live.state.caps.num_inputs as usize + 1;
                self.live
                    .handle_event(&mut self.session, ShellEvent::Identify(row));
            }
            fn run(&mut self, line: &str) {
                self.live.run_command(&mut self.session, line);
            }
            fn command(&mut self, line: &str) {
                self.live
                    .handle_event(&mut self.session, ShellEvent::Command(line.into()));
            }
            fn controls(&self) -> Vec<u16> {
                self.controls.lock().unwrap().clone()
            }
            fn configs(&self) -> usize {
                self.log
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e.opcode == op::REQ_SIGGEN_SET_CONFIG)
                    .count()
            }
            fn volumes(&self) -> Vec<Vec<u8>> {
                self.log
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e.opcode == op::REQ_SET_USER_VOLUME)
                    .map(|e| e.payload.clone())
                    .collect()
            }
            fn status(&self) -> String {
                self.live.shell.model.status.clone().unwrap_or_default()
            }
        }

        impl Drop for Rig {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }

        #[test]
        fn tm18_identify_stop_undo_sends_no_start() {
            let mut r = Rig::start("tm18-gen-stop-undo");
            r.identify();
            r.run("sig.control stop");
            r.run("undo");
            assert_eq!(r.controls(), [START, STOP]);
            assert_eq!(r.configs(), 1);
            assert!(r.status().contains("cannot be undone"), "{}", r.status());
        }

        #[test]
        fn tm18_identify_undo_redo_sends_no_start() {
            let mut r = Rig::start("tm18-gen-undo-redo");
            r.identify();
            r.run("undo");
            r.run("redo");
            assert_eq!(r.controls(), [START]);
            assert_eq!(r.configs(), 1);
        }

        #[test]
        fn tm18_ctrl_z_and_ctrl_y_send_no_start() {
            let mut r = Rig::start("tm18-gen-keys");
            r.identify();
            r.run("sig.control stop");
            let press = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
            for c in ['z', 'y', 'z', 'z', 'y'] {
                r.live.handle_key(&mut r.session, press(c));
            }
            assert_eq!(r.controls(), [START, STOP]);
        }

        /// Undo goes past the generator to the edit before it, and redo brings
        /// back only that edit.
        #[test]
        fn tm18_undo_after_a_sig_write_undoes_the_edit_before_it() {
            let mut r = Rig::start("tm18-gen-edit");
            r.run("vol.user -6");
            let edit = (-6.0f32).to_le_bytes().to_vec();
            assert_eq!(r.volumes(), std::slice::from_ref(&edit));
            r.identify();
            r.run("undo");
            assert_eq!(r.volumes().len(), 2, "the edit was put back");
            assert_ne!(r.volumes()[1], edit);
            assert!(r.status().contains("sig.control"), "{}", r.status());
            r.run("redo");
            assert_eq!(r.volumes().last(), Some(&edit));
            assert_eq!(r.controls(), [START]);
        }

        #[test]
        fn tm18_stop_on_a_running_generator_is_not_called_refused() {
            for (line, value) in [("sig.control stop", STOP), ("sig.control stop-now", 2)] {
                let mut r = Rig::start("tm18-gen-stop");
                r.identify();
                r.live.shell.model.status = None;
                r.run(line);
                assert_eq!(r.controls(), [START, value]);
                assert!(!r.status().contains("not applied"), "{}", r.status());
            }
        }

        /// A Stop from a panel goes straight out, past a volume write the gate
        /// is holding.
        #[test]
        fn tm18_stop_does_not_wait_on_the_write_gate() {
            let mut r = Rig::start("tm18-gen-gate");
            r.identify();
            r.live.clock = Some(Instant::now());
            r.command("vol.user -11");
            r.command("vol.user -12");
            assert_eq!(r.live.gate.held.len(), 1, "the second is held");
            r.command("sig.control stop");
            assert_eq!(r.controls(), [START, STOP]);
        }
    }
}

/// TM19: a Save of several flash writes through the real `Live`, on a demo
/// device that can be told to refuse one opcode.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm19 {
    use super::*;
    use crate::theme::ColorDepth;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::{
        DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps, TransportError,
    };
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::sync::{Arc, Mutex};

    /// The opcodes that write what a Save writes.
    const WRITES: [u8; 5] = [
        op::REQ_PRESET_SET_STARTUP,
        op::REQ_SET_MASTER_VOLUME_MODE,
        op::REQ_SET_OUTPUT_CONFIG_MODE,
        op::REQ_SAVE_OUTPUT_CONFIG,
        op::REQ_CS_SAVE,
    ];

    #[derive(Clone, Default)]
    struct Wire {
        /// Every write opcode attempted, refused or not.
        sent: Arc<Mutex<Vec<u8>>>,
        refuse: Arc<Mutex<Option<u8>>>,
    }

    impl Wire {
        fn see(&self, opcode: u8) -> dspi_transport::Result<()> {
            if WRITES.contains(&opcode) {
                self.sent.lock().unwrap().push(opcode);
            }
            if *self.refuse.lock().unwrap() == Some(opcode) {
                return Err(TransportError::Stalled { opcode });
            }
            Ok(())
        }
        fn sent(&self) -> Vec<u8> {
            self.sent.lock().unwrap().clone()
        }
    }

    /// The demo device, with a hand on the wire.
    struct Tapped(FakeDevice, Wire);

    impl Transport for Tapped {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.1.see(opcode)?;
            // -- WP QD31 -- The fake answers the output config save itself now.
            // The fake has no flash behind this one; the real device answers
            // FLASH_OK once it has written (vendor_commands).
            if opcode == op::REQ_CS_SAVE {
                return Ok(vec![0; usize::from(len)]);
            }
            self.0.control_in(opcode, value, len)
        }
        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            max_len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.1.see(opcode)?;
            self.0.control_in_upto(opcode, value, max_len)
        }
        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            self.1.see(opcode)?;
            self.0.control_out(opcode, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.0.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            self.0.notifications()
        }
        fn caps(&self) -> TransportCaps {
            self.0.caps()
        }
    }

    fn on_demo() -> (Live, Session, Wire) {
        let wire = Wire::default();
        let mut t: Box<dyn Transport> = Box::new(Tapped(FakeDevice::demo(), wire.clone()));
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let ext = ExtState::probe(t.as_mut()).unwrap();
        let mut session = Session::new(t, caps.clone()).unwrap();
        let bulk = session.snapshot().unwrap();
        let mut live = Live::new(
            DeviceState::new(caps, bulk),
            Theme::console(ColorDepth::TrueColor, Glyphs::Braille),
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        live.set_connection(&Target::Demo, &ext);
        (live, session, wire)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn frame(l: &mut Live) -> String {
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        l.draw(area, &mut buf);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Settings, on Global Parameters, with the focus in the page.
    fn global_page(l: &mut Live, s: &mut Session) {
        l.show(s, "settings");
        for _ in 0..20 {
            if frame(l).contains("STARTUP PRESET") {
                l.handle_key(s, key(KeyCode::Tab));
                return;
            }
            l.handle_key(s, key(KeyCode::Down));
        }
        panic!("no Global page:\n{}", frame(l));
    }

    /// Move a two-way picker off the value it is on.
    fn flip(l: &mut Live, s: &mut Session, on: bool) {
        l.handle_key(s, key(if on { KeyCode::Left } else { KeyCode::Right }));
    }

    /// Stage a change to the startup mode, the row after the device name the
    /// page opens on.
    fn edit_startup(l: &mut Live, s: &mut Session) {
        l.handle_key(s, key(KeyCode::Down));
        let on = device(s).0.first() == Some(&1);
        flip(l, s, on);
    }

    /// Stage a change to both persistence modes, the page's last two rows.
    fn edit_modes(l: &mut Live, s: &mut Session) {
        let (_, master, io) = device(s);
        for _ in 0..30 {
            l.handle_key(s, key(KeyCode::Down));
        }
        flip(l, s, io == Value::Choice(1));
        l.handle_key(s, key(KeyCode::Up));
        flip(l, s, master == Value::Choice(1));
    }

    fn save(l: &mut Live, s: &mut Session) {
        l.handle_key(s, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    }

    /// The device's values for the three the page edits here. `read` asks for
    /// one byte of the startup packet, so that one is read whole.
    fn device(s: &mut Session) -> (Vec<u8>, Value, Value) {
        (
            s.with_transport(|t| t.control_in(op::REQ_PRESET_GET_STARTUP, 0, 2))
                .unwrap(),
            s.read("vol.master.mode", &[]).unwrap(),
            s.read("preset.iomode", &[]).unwrap(),
        )
    }

    #[test]
    fn wp_tm19_one_confirmation_names_every_line_and_writes_them_in_order() {
        let (mut l, mut s, wire) = on_demo();
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);
        edit_modes(&mut l, &mut s);
        let before = device(&mut s);
        save(&mut l, &mut s);
        assert!(
            matches!(&l.dialog, Some((AppDialog::HazardBlock { lines, .. }, _)) if lines.len() == 3),
            "{:?}",
            l.dialog
        );
        let f = frame(&mut l);
        for path in ["preset.startup", "vol.master.mode", "preset.iomode"] {
            assert!(f.contains(path), "{path} is not named:\n{f}");
        }
        assert!(wire.sent().is_empty(), "nothing goes before the answer");

        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.dialog.is_none(), "{:?}", l.dialog);
        assert_eq!(
            wire.sent(),
            vec![
                op::REQ_PRESET_SET_STARTUP,
                op::REQ_SET_MASTER_VOLUME_MODE,
                op::REQ_SET_OUTPUT_CONFIG_MODE
            ],
            "every line, in the order Save gave them"
        );
        let after = device(&mut s);
        assert_ne!(before.0, after.0);
        assert_ne!(before.1, after.1);
        assert_ne!(before.2, after.2);

        save(&mut l, &mut s);
        assert!(l.dialog.is_none());
        assert_eq!(l.shell.model.status.as_deref(), Some("Nothing to save"));
    }

    /// The probe's block, typed as Save's own grammar: the first line used to
    /// be lost, with only the last line's confirmation surviving.
    #[test]
    fn wp_tm19_the_first_line_of_a_block_is_not_dropped() {
        let (mut l, mut s, _) = on_demo();
        let (startup, _, iomode) = device(&mut s);
        let want = if iomode == Value::Choice(1) {
            "independent"
        } else {
            "with-preset"
        };
        l.run_commands(
            &mut s,
            &format!("preset.startup mode=specified slot=3\npreset.iomode {want}"),
        );
        let (kind, _) = l.dialog.take().unwrap();
        l.finish_dialog(&mut s, kind, DialogOutcome::Button(0));
        assert!(l.dialog.is_none(), "nothing further to ask");
        let (startup_now, _, iomode_now) = device(&mut s);
        assert_ne!(startup, startup_now);
        assert_ne!(iomode, iomode_now);
    }

    /// Every line Save can hold fits: a clipped dialog would ask about writes
    /// it does not show.
    #[test]
    fn wp_tm19_the_longest_save_is_named_whole_on_a_small_terminal() {
        let (mut l, mut s, wire) = on_demo();
        l.run_commands(
            &mut s,
            "preset.startup mode=specified slot=3\nvol.master.mode independent\n\
             preset.iomode independent\n\
             dev.dacmute enabled=on active_low=off pin=4 hold_ms=20 release_ms=20\n\
             out.latency 1\ndev.save.io\ncs.save",
        );
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        l.draw(area, &mut buf);
        let f: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|p| buf[p].symbol().to_string())
            .collect();
        for path in [
            "preset.startup",
            "vol.master.mode",
            "preset.iomode",
            "dev.dacmute",
            "out.latency",
            "dev.save.io",
            "cs.save",
            "This cannot be undone.",
        ] {
            assert!(f.contains(path), "{path} is not shown");
        }
        assert!(wire.sent().is_empty());
    }

    #[test]
    fn wp_tm19_cancel_writes_nothing_and_keeps_the_draft_dirty() {
        let (mut l, mut s, wire) = on_demo();
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);
        edit_modes(&mut l, &mut s);
        let before = device(&mut s);
        for answer in [key(KeyCode::Esc), key(KeyCode::Tab)] {
            save(&mut l, &mut s);
            assert!(matches!(l.dialog, Some((AppDialog::HazardBlock { .. }, _))));
            l.handle_key(&mut s, answer);
            if l.dialog.is_some() {
                // Tab moved to Cancel; press it.
                l.handle_key(&mut s, key(KeyCode::Enter));
            }
            assert!(l.dialog.is_none());
            assert_eq!(
                l.shell.model.status.as_deref(),
                Some("Cancelled; nothing was written")
            );
        }
        assert!(wire.sent().is_empty());
        assert_eq!(device(&mut s), before);
        // Still all there: the same three lines are asked about again.
        save(&mut l, &mut s);
        assert!(
            matches!(&l.dialog, Some((AppDialog::HazardBlock { lines, .. }, _)) if lines.len() == 3)
        );
    }

    #[test]
    fn wp_tm19_a_failure_partway_reports_both_and_keeps_the_rest_in_the_draft() {
        let (mut l, mut s, wire) = on_demo();
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);
        edit_modes(&mut l, &mut s);
        let before = device(&mut s);
        *wire.refuse.lock().unwrap() = Some(op::REQ_SET_MASTER_VOLUME_MODE);
        save(&mut l, &mut s);
        l.handle_key(&mut s, key(KeyCode::Enter));

        // The report says who was written and who was not.
        let Some((AppDialog::Report, dialog)) = &l.dialog else {
            panic!("{:?}", l.dialog);
        };
        let crate::widgets::DialogKind::Report { lines, .. } = &dialog.kind else {
            panic!("{:?}", dialog.kind);
        };
        let at = |needle: &str| lines.iter().position(|x| x.contains(needle)).unwrap();
        assert!(at("Written:") < at("preset.startup"), "{lines:?}");
        assert!(at("preset.startup") < at("Not written:"), "{lines:?}");
        assert!(at("Not written:") < at("vol.master.mode"), "{lines:?}");
        assert!(at("vol.master.mode") < at("preset.iomode"), "{lines:?}");
        assert!(lines.iter().any(|x| x.contains("stalled")), "{lines:?}");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.dialog.is_none());

        // Stopped at the refusal, once, with the third line never sent.
        assert_eq!(
            wire.sent(),
            vec![op::REQ_PRESET_SET_STARTUP, op::REQ_SET_MASTER_VOLUME_MODE]
        );
        let now = device(&mut s);
        assert_ne!(now.0, before.0);
        assert_eq!(now.1, before.1);
        assert_eq!(now.2, before.2);

        // The next Save asks about the two that are left, and not the written one.
        wire.sent.lock().unwrap().clear();
        *wire.refuse.lock().unwrap() = None;
        save(&mut l, &mut s);
        let Some((AppDialog::HazardBlock { lines, .. }, _)) = &l.dialog else {
            panic!("{:?}", l.dialog);
        };
        let paths: Vec<&str> = lines.iter().map(|w| w.path).collect();
        assert_eq!(paths, ["vol.master.mode", "preset.iomode"]);
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(
            wire.sent(),
            vec![
                op::REQ_SET_MASTER_VOLUME_MODE,
                op::REQ_SET_OUTPUT_CONFIG_MODE
            ]
        );
        save(&mut l, &mut s);
        assert_eq!(l.shell.model.status.as_deref(), Some("Nothing to save"));
    }

    #[test]
    fn wp_tm19_a_failure_on_the_first_line_says_nothing_was_written() {
        let (mut l, mut s, wire) = on_demo();
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);
        edit_modes(&mut l, &mut s);
        *wire.refuse.lock().unwrap() = Some(op::REQ_PRESET_SET_STARTUP);
        save(&mut l, &mut s);
        l.handle_key(&mut s, key(KeyCode::Enter));
        let Some((AppDialog::Report, dialog)) = &l.dialog else {
            panic!("{:?}", l.dialog);
        };
        let crate::widgets::DialogKind::Report { lines, .. } = &dialog.kind else {
            panic!("{:?}", dialog.kind);
        };
        assert_eq!(lines[1], "  nothing", "{lines:?}");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(wire.sent(), vec![op::REQ_PRESET_SET_STARTUP]);
        save(&mut l, &mut s);
        assert!(
            matches!(&l.dialog, Some((AppDialog::HazardBlock { lines, .. }, _)) if lines.len() == 3),
            "the draft is whole"
        );
    }

    /// The output-config category too counts only once its flash went through.
    #[test]
    fn wp_tm19_the_output_config_stays_dirty_until_its_own_line_is_written() {
        let (mut l, mut s, wire) = on_demo();
        l.state.output_config_mode = 0;
        l.run_command(&mut s, "limit.threshold 0 -6");
        assert!(l.state.limiter_unsaved());
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);

        *wire.refuse.lock().unwrap() = Some(op::REQ_SAVE_OUTPUT_CONFIG);
        save(&mut l, &mut s);
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(matches!(l.dialog, Some((AppDialog::Report, _))));
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.state.limiter_unsaved());

        *wire.refuse.lock().unwrap() = None;
        save(&mut l, &mut s);
        let Some((AppDialog::Hazard { path, .. }, _)) = &l.dialog else {
            panic!("only the flash is left, so one line: {:?}", l.dialog);
        };
        assert_eq!(path, "dev.save.io");
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(!l.state.limiter_unsaved());
        save(&mut l, &mut s);
        assert_eq!(l.shell.model.status.as_deref(), Some("Nothing to save"));
    }

    /// One line keeps the old dialog and its wording; only what it means for
    /// the draft changed.
    #[test]
    fn wp_tm19_a_single_line_keeps_its_own_confirmation() {
        let (mut l, mut s, wire) = on_demo();
        global_page(&mut l, &mut s);
        edit_startup(&mut l, &mut s);
        save(&mut l, &mut s);
        let Some((AppDialog::Hazard { path, .. }, dialog)) = &l.dialog else {
            panic!("{:?}", l.dialog);
        };
        assert_eq!(path, "preset.startup");
        assert_eq!(dialog.title, "Are you sure?");
        assert!(
            dialog.body.starts_with("`preset.startup`: "),
            "{}",
            dialog.body
        );
        assert!(
            dialog.body.ends_with("This cannot be undone."),
            "{}",
            dialog.body
        );

        // Cancel: nothing sent, still dirty.
        l.handle_key(&mut s, key(KeyCode::Esc));
        assert!(wire.sent().is_empty());
        save(&mut l, &mut s);
        assert!(matches!(l.dialog, Some((AppDialog::Hazard { .. }, _))));

        // A refusal: the note, no report, still dirty.
        *wire.refuse.lock().unwrap() = Some(op::REQ_PRESET_SET_STARTUP);
        l.handle_key(&mut s, key(KeyCode::Enter));
        assert!(l.dialog.is_none(), "{:?}", l.dialog);
        assert!(
            l.shell.model.status.as_deref().unwrap().contains("stalled"),
            "{:?}",
            l.shell.model.status
        );
        *wire.refuse.lock().unwrap() = None;
        save(&mut l, &mut s);
        assert!(matches!(l.dialog, Some((AppDialog::Hazard { .. }, _))));

        // Confirmed: written and clean.
        l.handle_key(&mut s, key(KeyCode::Enter));
        save(&mut l, &mut s);
        assert!(l.dialog.is_none());
        assert_eq!(l.shell.model.status.as_deref(), Some("Nothing to save"));
    }

    /// A `#` note is not a command: it does not make a block, and it is left
    /// on the status line once the block has gone.
    #[test]
    fn wp_tm19_a_note_is_not_a_line_and_lands_after_the_block() {
        let (mut l, mut s, _) = on_demo();
        l.run_commands(&mut s, "dev.save.io\n# Saved");
        assert!(matches!(l.dialog, Some((AppDialog::Hazard { .. }, _))));
        l.dialog = None;

        l.run_commands(&mut s, "preset.iomode independent\ndev.save.io\n# Done");
        let (kind, _) = l.dialog.take().unwrap();
        assert_ne!(l.shell.model.status.as_deref(), Some("Done"));
        l.finish_dialog(&mut s, kind, DialogOutcome::Button(0));
        assert_eq!(l.shell.model.status.as_deref(), Some("Done"));
    }

    /// A line with no result to report cannot join a confirmed block.
    #[test]
    fn wp_tm19_a_block_with_a_bad_line_writes_nothing() {
        let (mut l, mut s, wire) = on_demo();
        l.run_commands(&mut s, "preset.iomode independent\nno.such.thing 1");
        assert!(l.dialog.is_none());
        assert!(
            l.shell
                .model
                .status
                .as_deref()
                .unwrap()
                .contains("no.such.thing"),
            "{:?}",
            l.shell.model.status
        );
        assert!(wire.sent().is_empty());
    }

    /// A write held back by the rate gate is not part of what is asked about,
    /// and is not lost when the answer is no.
    #[test]
    fn wp_tm19_a_held_write_goes_first_and_is_not_in_the_block() {
        let (mut l, mut s, wire) = on_demo();
        l.clock = Some(Instant::now());
        l.handle_event(&mut s, ShellEvent::Command("vol.user -11".into()));
        l.handle_event(&mut s, ShellEvent::Command("vol.user -12".into()));
        assert_eq!(l.gate.held.len(), 1, "the second is within the interval");
        l.handle_event(
            &mut s,
            ShellEvent::Command(
                "preset.startup mode=specified slot=3\npreset.iomode independent".into(),
            ),
        );
        assert!(
            matches!(&l.dialog, Some((AppDialog::HazardBlock { lines, .. }, _)) if lines.len() == 2),
            "{:?}",
            l.dialog
        );
        assert!(l.gate.held.is_empty());
        assert_eq!(s.read("vol.user", &[]).unwrap(), Value::Float(-12.0));
        assert!(wire.sent().is_empty());
    }

    /// A block with nothing to confirm runs as it always has.
    #[test]
    fn wp_tm19_a_block_without_a_hazard_is_not_asked_about() {
        let (mut l, mut s, _) = on_demo();
        l.run_commands(&mut s, "vol.user -11\nvol.user -12");
        assert!(l.dialog.is_none());
        assert_eq!(s.read("vol.user", &[]).unwrap(), Value::Float(-12.0));
    }
}

// -- WP TM5 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm5 {
    use std::thread::sleep;

    use dspi_link::fakehub::{FakeHub, HubConfig, req};
    use ratatui::style::Modifier;

    use super::*;
    use crate::shell::connect::{LinkTarget, test_dir};
    use crate::theme::{ColorDepth, Glyphs};
    use crate::widgets::testing::key;
    use ratatui::buffer::Buffer;

    fn hub() -> FakeHub {
        FakeHub::start(HubConfig {
            rta: true,
            ..HubConfig::default()
        })
        .unwrap()
    }

    /// A terminal on `hub`, as `uconsole --link` starts one.
    struct Console {
        live: Live,
        session: Session,
    }

    impl Console {
        fn on(hub: &FakeHub, name: &str) -> Self {
            let dir = test_dir().join(format!("tm5-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut link = LinkTarget::parse(&hub.addr().to_string()).unwrap();
            link.tokens = Some(dir.join("tokens.toml"));
            let target = Target::Link(link);
            let c = target.connect(&|_: &str, _: &str| None).unwrap();
            let serial = c.state.caps.serial.clone();
            let mut live = Live::new(
                c.state,
                Theme::console(ColorDepth::TrueColor, Glyphs::Braille),
                Performance::lite(),
                Box::new(ConsoleScreens::new()),
            );
            let mut session = c.session;
            live.set_connection(&target.pinned(&serial), &c.ext);
            live.arm_notifications(&mut session);
            live.connect_analyser(&mut session);
            Self { live, session }
        }

        fn tick(&mut self) {
            let notes = self.live.notes.take();
            self.live.tick(&mut self.session, notes.as_ref());
            self.live.notes = notes;
            // As the run loop does: a frame between ticks.
            let area = ratatui::layout::Rect::new(0, 0, 140, 45);
            self.live.draw(area, &mut Buffer::empty(area));
        }

        fn press(&mut self, c: char) {
            let k = key(KeyCode::Char(c));
            self.live.handle_key(&mut self.session, k);
        }

        fn sid(&mut self) -> u16 {
            self.session
                .with_transport(|t| Ok(t.session_id()))
                .unwrap()
                .unwrap()
        }

        fn wanted_mask(&self) -> Option<u16> {
            let app = self.live.shared.borrow();
            app.spectrum.engine.wanted().map(|w| w.channel_mask)
        }

        fn following(&self) -> Option<u16> {
            let app = self.live.shared.borrow();
            app.spectrum.sharing.following().map(|f| f.origin)
        }

        /// Choose outputs as the panel's chips do.
        fn choose(&mut self, channels: &[u8]) {
            self.live.shared.borrow_mut().spectrum.selection =
                Some(crate::screens::spectrum::Selection::new(
                    dspi_session::rta::TAP_OUTPUT,
                    channels.to_vec(),
                ));
        }
    }

    fn until(
        what: &str,
        consoles: &mut [&mut Console],
        mut f: impl FnMut(&[&mut Console]) -> bool,
    ) {
        for _ in 0..800 {
            for c in consoles.iter_mut() {
                c.tick();
            }
            if f(consoles) {
                return;
            }
            sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}");
    }

    fn settle(consoles: &mut [&mut Console]) {
        for _ in 0..30 {
            for c in consoles.iter_mut() {
                c.tick();
            }
            sleep(Duration::from_millis(5));
        }
    }

    fn pushes(hub: &FakeHub, sid: u16) -> usize {
        hub.log()
            .iter()
            .filter(|e| {
                e.0 == sid && e.1 == dspi_link::frames::Dir::Set as u8 && e.2 == req::RTA_SET_CONFIG
            })
            .count()
    }

    fn device_mask(hub: &FakeHub) -> u16 {
        let c = hub.rta_config();
        u16::from_le_bytes([c[2], c[3]])
    }

    // -- WP TM6 --
    #[test]
    fn wp_tm6_capital_f_shows_the_bars_and_runs_the_analyser_on_the_fake_hub() {
        let hub = hub();
        let mut a = Console::on(&hub, "bars-a");
        a.choose(&[0, 1]);
        settle(&mut [&mut a]);
        assert_eq!(a.wanted_mask(), None, "nothing watches yet");

        a.press('F');
        until("the analyser to run", &mut [&mut a], |c| {
            c[0].wanted_mask() == Some(0b11) && c[0].live.shell.model.bars.is_some()
        });
        until("the device to take it", &mut [&mut a], |_| {
            device_mask(&hub) == 0b11
        });
        let p = a.live.shell.model.bars.clone().unwrap();
        assert_eq!(p.channels.len(), 2);

        // A tool covers the page: the bars stop watching and the analyser stops.
        a.press('L');
        until("the bars to give way", &mut [&mut a], |c| {
            c[0].live.shell.model.bars.is_none()
        });
        a.press('L');
        until("the bars to return", &mut [&mut a], |c| {
            c[0].live.shell.model.bars.is_some()
        });

        a.press('F');
        until("the bars to go", &mut [&mut a], |c| {
            c[0].wanted_mask().is_none() && c[0].live.shell.model.bars.is_none()
        });
    }

    #[test]
    fn wp_tm5_following_and_the_take_control_key_on_the_fake_hub() {
        let hub = hub();
        let mut a = Console::on(&hub, "follow-a");
        let sid_a = a.sid();
        a.choose(&[0]);
        a.press('A'); // the panel
        until("a's push", &mut [&mut a], |_| device_mask(&hub) == 0b01);
        // Our own echo arrives and changes nothing.
        settle(&mut [&mut a]);
        assert_eq!(a.following(), None);

        // Another session: a follows, with the origin, and polls what it set.
        let mut b = Console::on(&hub, "follow-b");
        let sid_b = b.sid();
        b.choose(&[1]);
        b.press('A');
        until("a following", &mut [&mut a, &mut b], |c| {
            c[0].following().is_some()
        });
        assert_eq!(a.following(), Some(sid_b));
        until("a's picture", &mut [&mut a, &mut b], |c| {
            c[0].wanted_mask() == Some(0b10)
        });
        // A follower never pushes, and the driver stays the driver.
        let n = pushes(&hub, sid_a);
        settle(&mut [&mut a, &mut b]);
        assert_eq!(pushes(&hub, sid_a), n, "a follower does not push");
        assert_eq!(b.following(), None);
        assert_eq!(device_mask(&hub), 0b10);

        // The key: a pushes its own again, and b follows now.
        a.press('t');
        assert_eq!(a.following(), None);
        until("a's push again", &mut [&mut a, &mut b], |_| {
            device_mask(&hub) == 0b01
        });
        until("b following", &mut [&mut a, &mut b], |c| {
            c[1].following().is_some()
        });
        assert_eq!(b.following(), Some(sid_a));
        settle(&mut [&mut a, &mut b]);
        assert_eq!(a.following(), None, "ours does not switch state");
        assert_eq!(device_mask(&hub), 0b01, "no fight");
    }

    #[test]
    fn wp_tm5_the_overlay_draws_on_the_channel_graph_on_the_fake_hub() {
        let hub = hub();
        let mut a = Console::on(&hub, "overlay");
        let sid = a.sid();
        settle(&mut [&mut a]);
        assert_eq!(pushes(&hub, sid), 0, "off until asked for");

        a.live
            .handle_event(&mut a.session, ShellEvent::Select(Selection::Output(1)));
        a.press('f');
        until("frames", &mut [&mut a], |c| {
            c[0].live.shell.model.spectrum.as_ref().is_some_and(|o| {
                o.curves
                    .iter()
                    .any(|g| g.magnitude.iter().any(|v| v.is_finite()))
            })
        });
        assert_eq!(device_mask(&hub), 0b10, "the graphed channel");
        let area = ratatui::layout::Rect::new(0, 0, 140, 45);
        let mut buf = Buffer::empty(area);
        a.live.draw(area, &mut buf);
        let dim_braille = buf.content().iter().any(|c| {
            c.modifier.contains(Modifier::DIM)
                && c.symbol()
                    .chars()
                    .next()
                    .is_some_and(|ch| ('\u{2800}'..='\u{28FF}').contains(&ch))
        });
        assert!(dim_braille, "the spectrum is on the graph");

        // Back in the overview the graph is gone, so the analyser stops.
        a.live
            .handle_event(&mut a.session, ShellEvent::Select(Selection::Overview));
        until("released", &mut [&mut a], |c| {
            c[0].live.shell.model.spectrum.is_none() && c[0].wanted_mask().is_none()
        });
        a.press('f');
        settle(&mut [&mut a]);
        assert!(a.live.shell.model.spectrum.is_none());
    }

    #[test]
    fn wp_tm5_the_overlay_comes_back_after_a_reconnect_with_its_graph() {
        let hub = hub();
        let mut a = Console::on(&hub, "reconnect-overlay");
        let first = a.sid();
        a.live
            .handle_event(&mut a.session, ShellEvent::Select(Selection::Output(0)));
        a.press('f');
        until("the overlay's push", &mut [&mut a], |_| {
            pushes(&hub, first) > 0
        });

        hub.close_all(1001);
        until("the loss", &mut [&mut a], |c| !c[0].live.state.connected);
        for _ in 0..1000 {
            a.live.advance_reconnect(&mut a.session);
            if a.live.state.connected {
                break;
            }
            sleep(Duration::from_millis(5));
        }
        assert!(a.live.state.connected, "came back");
        let second = a.sid();
        // Losing the device left the channel page, and the graph with it.
        settle(&mut [&mut a]);
        assert_eq!(pushes(&hub, second), 0, "nothing on screen to feed");

        // Its page again, with no `f`: it was on, so it runs.
        a.live
            .handle_event(&mut a.session, ShellEvent::Select(Selection::Output(0)));
        until("the analyser back", &mut [&mut a], |c| {
            pushes(&hub, second) > 0 && c[0].live.shell.model.spectrum.is_some()
        });
    }

    #[test]
    fn wp_tm5_a_reconnect_starts_the_analyser_again_and_forgets_who_was_followed() {
        let hub = hub();
        let mut a = Console::on(&hub, "reconnect-a");
        let first = a.sid();
        a.choose(&[0]);
        a.press('A');
        until("a's push", &mut [&mut a], |_| pushes(&hub, first) > 0);
        let mut b = Console::on(&hub, "reconnect-b");
        b.choose(&[1]);
        b.press('A');
        until("a following", &mut [&mut a, &mut b], |c| {
            c[0].following().is_some()
        });
        drop(b);

        hub.close_all(1001);
        until("the loss", &mut [&mut a], |c| !c[0].live.state.connected);
        for _ in 0..1000 {
            a.live.advance_reconnect(&mut a.session);
            if a.live.state.connected {
                break;
            }
            sleep(Duration::from_millis(5));
        }
        assert!(a.live.state.connected, "came back");
        let second = a.sid();
        assert_ne!(first, second);
        assert_eq!(a.following(), None, "that belonged to the old connection");

        // Nobody touched anything: it was running, so it runs.
        until("the analyser back", &mut [&mut a], |_| {
            pushes(&hub, second) > 0
                && hub
                    .log()
                    .iter()
                    .any(|e| e.0 == second && e.2 == req::RTA_GET_BANDS)
        });
        assert_eq!(device_mask(&hub), 0b01, "our own choice");
    }
}

// -- WP TM12 --
#[cfg(test)]
mod tests_wp_tm12 {
    use super::*;

    /// A verb of both the grammar and the interface is offered once.
    #[test]
    fn tm12_the_palette_lists_autoeq_once() {
        let ctx = Context {
            channel_slugs: Vec::new(),
            num_inputs: 0,
            num_outputs: 0,
            max_bands: 0,
        };
        let prompt = Prompt::new(true, &ctx);
        let listed = prompt
            .candidates
            .iter()
            .filter(|c| c.value == "autoeq")
            .count();
        assert_eq!(listed, 1);
    }
}

// -- WP SECTM --
#[cfg(test)]
mod tests_wp_sectm {
    use super::*;
    use crate::shell::connect::LinkTarget;
    use crate::theme::{ColorDepth, Glyphs};
    use dspi_transport::{FakeDevice, Transport};
    use ratatui::buffer::Buffer;

    /// A live interface on a demo device, connected "through" `target`.
    fn on(target: &Target, width: u16) -> (Live, String, Buffer) {
        let mut t: Box<dyn Transport> = Box::new(FakeDevice::demo());
        let caps = dspi_session::probe(t.as_mut()).unwrap();
        let ext = ExtState::probe(t.as_mut()).unwrap();
        let mut session = Session::new(t, caps.clone()).unwrap();
        let bulk = session.snapshot().unwrap();
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let mut live = Live::new(
            DeviceState::new(caps, bulk),
            theme,
            Performance::default(),
            Box::new(ConsoleScreens::new()),
        );
        live.set_connection(target, &ext);
        live.sync_model();
        let area = Rect::new(0, 0, width, 40);
        let mut buf = Buffer::empty(area);
        live.draw(area, &mut buf);
        let top: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        (live, top, buf)
    }

    fn hub(encrypted: Option<bool>) -> Target {
        Target::Link(LinkTarget {
            encrypted,
            ..LinkTarget::parse("studio.local").unwrap()
        })
    }

    #[test]
    fn wp_sectm_an_unencrypted_hub_session_says_unsecured_in_red() {
        let (live, top, buf) = on(&hub(Some(false)), 140);
        assert!(live.shell.model.link_unsecured);
        assert_eq!(live.shell.model.via, "Link studio.local (unsecured)");
        assert!(top.contains("· Link studio.local (unsecured)"), "{top}");
        // Red, and the words carry it without colour too
        let at = top.find("(unsecured)").unwrap();
        let x = top.get(..at).unwrap().chars().count() as u16;
        let danger = live.shell.theme.danger;
        for dx in 0.."(unsecured)".len() as u16 {
            assert_eq!(buf[(x + dx, 0)].fg, danger, "{top}");
        }
        assert_ne!(buf[(x.saturating_sub(3), 0)].fg, danger, "only the badge");
        // Said once, with the calm explanation
        let status = live.shell.model.status.clone().unwrap_or_default();
        assert!(
            status.contains("trusted local network or over a VPN"),
            "{status}"
        );
    }

    #[test]
    fn wp_sectm_an_encrypted_or_unknown_session_has_no_badge() {
        for encrypted in [Some(true), None] {
            let (live, top, _) = on(&hub(encrypted), 140);
            assert!(!live.shell.model.link_unsecured);
            assert_eq!(live.shell.model.via, "Link studio.local");
            assert!(!top.contains("unsecured"), "{top}");
        }
        let (live, _, _) = on(&Target::Demo, 140);
        assert!(!live.shell.model.link_unsecured);
    }

    // -- WP SECFIX --
    #[test]
    fn wp_secfix_a_plaintext_session_on_loopback_has_no_badge() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            let t = Target::Link(LinkTarget {
                encrypted: Some(false),
                ..LinkTarget::parse(host).unwrap()
            });
            let (live, top, _) = on(&t, 140);
            assert!(!live.shell.model.link_unsecured, "{host}");
            assert!(!top.contains("unsecured"), "{host}: {top}");
            let status = live.shell.model.status.clone().unwrap_or_default();
            assert!(!status.contains("VPN"), "no explanation either: {status}");
        }
    }

    #[test]
    fn wp_sectm_a_narrow_title_keeps_the_badge() {
        // Where the link's name no longer fits, the badge still does
        let narrow = (60..140)
            .rev()
            .map(|w| on(&hub(Some(false)), w).1)
            .find(|top| !top.contains("Link studio.local"))
            .expect("some width drops the link's name");
        assert!(narrow.contains("· (unsecured)"), "{narrow}");
        assert!(narrow.contains("Connected"), "{narrow}");
    }
}
