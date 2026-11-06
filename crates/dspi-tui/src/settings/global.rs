//! Settings > Global Parameters.
//!
//! Everything that lives in the device's flash directory rather than in a
//! preset. Unlike every other page, edits here are staged in a local draft and
//! not written until Save, so the directory sector is not re-flashed on every
//! picker change. That is the Console's `GlobalSettingsDraft`, and the save
//! bar's first dirty category.
//!
//! The device name is the exception: its dialog's OK is the commit, and it
//! writes at once.

use std::rc::Rc;

use dspi_proto::packets::DacHwMuteConfig;
use dspi_proto::value::Value;
use dspi_session::{DeviceState, Session};

use crate::shell::SessionReply;
use crate::widgets::text::truncate;
use crate::widgets::{Action, Dialog, DialogOutcome, KeyHelp, PopupList};

use super::pin_status as st;
use super::{Cx, PageEvent, Row, SettingsData, SettingsPage};

/// `INPUT_SOURCE_NAME_LEN` / `DEVICE_NAME_LEN` less the NUL.
const NAME_MAX: usize = 31;
/// `INPUT_SOURCE_MAX` + 1: USB, S/PDIF 1, I2S, ADAT, S/PDIF 2-4.
const SOURCE_COUNT: u8 = 7;
/// The most columns a stored name takes on its row's button, so a full-length
/// name leaves room for the label on a narrow page.
pub(crate) const NAME_BUTTON_WIDTH: usize = 24;

/// `OUTPUT_LATENCY_GET_STATUS` / `OUTPUT_LATENCY_GET_OPTIONS`, the IN `wValue`s
/// of `REQ_OUTPUT_LATENCY` (config.h:261-262).
const LATENCY_GET_STATUS: u16 = 0;
const LATENCY_GET_OPTIONS: u16 = 1;
const LATENCY_STATUS_LEN: u16 = 12;
const LATENCY_OPTION_LEN: usize = 8;
/// `OUTPUT_LATENCY_DEFAULT`, the 4 ms option (output_latency.h).
const LATENCY_DEFAULT_ID: u8 = 5;
/// `OUTPUT_LATENCY_WIRE_AVAILABLE` (config.h:1214).
const LATENCY_AVAILABLE: u8 = 0x80;

/// One row of the device's output-latency table (`OutputLatencyOption`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyOption {
    pub id: u8,
    /// False when the active source runs this option as a different one.
    pub available: bool,
    pub latency_us: u16,
    /// What it actually gives at the current source and rate.
    pub effective_us: u16,
}

/// The output-latency setting and the options the device offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latency {
    pub requested_id: u8,
    pub options: Vec<LatencyOption>,
}

/// The directory settings that are in neither the bulk image nor any other
/// read: the board's name, the source names and the output latency.
///
/// Each stays absent on firmware without it, which is what hides its rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceExtras {
    pub device_name: Option<String>,
    /// By source id. Empty on firmware without source names; `None` for a
    /// source whose read failed.
    pub source_names: Vec<Option<String>>,
    pub latency: Option<Latency>,
}

impl DeviceExtras {
    /// Read whatever the connect-time probe found, and ask nothing of a
    /// firmware that failed it: an old one answers `0x57` as something else
    /// (device_name_spec.md section 2).
    pub fn read(session: &mut Session) -> Self {
        let has = |session: &Session, name: &str| {
            session
                .capabilities()
                .features
                .iter()
                .any(|f| f.name == name && f.present)
        };
        let device_name = if has(session, "device_name") {
            read_name(session, "dev.name", &[])
        } else {
            None
        };
        let mut source_names = Vec::new();
        // Source 0 is the Console's probe: without it there is no section.
        if has(session, "input_source_names")
            && let Some(first) = read_name(session, "in.name", &[0])
        {
            source_names.push(Some(first));
            for source in 1..SOURCE_COUNT {
                source_names.push(read_name(session, "in.name", &[source]));
            }
        }
        let latency = if has(session, "output_latency") {
            read_latency(session)
        } else {
            None
        };
        Self {
            device_name,
            source_names,
            latency,
        }
    }

    /// Take on a re-read. A read that failed says nothing about the device (a
    /// flash write blacks out the control endpoint), so it keeps what we had.
    pub fn adopt(&mut self, new: Self) {
        if new.device_name.is_some() {
            self.device_name = new.device_name;
        }
        if self.source_names.len() != new.source_names.len() {
            if !new.source_names.is_empty() {
                self.source_names = new.source_names;
            }
        } else {
            for (have, got) in self.source_names.iter_mut().zip(new.source_names) {
                if got.is_some() {
                    *have = got;
                }
            }
        }
        if let Some(mut latency) = new.latency {
            if latency.options.is_empty()
                && let Some(old) = self.latency.take()
            {
                latency.options = old.options;
            }
            self.latency = Some(latency);
        }
    }
}

fn read_name(session: &mut Session, path: &str, indices: &[u8]) -> Option<String> {
    match session.read(path, indices) {
        Ok(Value::Text(t)) => Some(clean_name(&t)),
        _ => None,
    }
}

fn read_latency(session: &mut Session) -> Option<Latency> {
    use dspi_proto::generated::opcodes as op;
    let status = session
        .with_transport(|t| {
            t.control_in(
                op::REQ_OUTPUT_LATENCY,
                LATENCY_GET_STATUS,
                LATENCY_STATUS_LEN,
            )
        })
        .ok()?;
    let (requested_id, count) = decode_latency_status(&status)?;
    let table_len = u16::from(count).saturating_mul(LATENCY_OPTION_LEN as u16);
    let options = session
        .with_transport(|t| t.control_in(op::REQ_OUTPUT_LATENCY, LATENCY_GET_OPTIONS, table_len))
        .map(|d| decode_latency_options(&d))
        .unwrap_or_default();
    Some(Latency {
        requested_id,
        options,
    })
}

/// `OutputLatencyStatus`: the requested id and the option count.
fn decode_latency_status(b: &[u8]) -> Option<(u8, u8)> {
    if b.len() < usize::from(LATENCY_STATUS_LEN) {
        return None;
    }
    Some((*b.first()?, *b.get(9)?))
}

/// The option table; a trailing partial entry is dropped.
fn decode_latency_options(b: &[u8]) -> Vec<LatencyOption> {
    b.as_chunks::<LATENCY_OPTION_LEN>()
        .0
        .iter()
        .map(|&[id, flags, l0, l1, _, _, e0, e1]| LatencyOption {
            id,
            available: flags & LATENCY_AVAILABLE != 0,
            latency_us: u16::from_le_bytes([l0, l1]),
            effective_us: u16::from_le_bytes([e0, e1]),
        })
        .collect()
}

/// `formatLatencyMs`: "0.67 ms", "4 ms", "3.66 ms".
fn latency_ms(us: u16) -> String {
    if us.is_multiple_of(1000) {
        format!("{} ms", us / 1000)
    } else {
        format!("{:.2} ms", f64::from(us) / 1000.0)
    }
}

/// What a name may hold, the Console's `sanitizeInputSourceName`: printable
/// ASCII, which is all the front panel's font has, and 31 of them. It also
/// keeps a name read from the device from putting escapes on the terminal.
pub(crate) fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| (' '..='~').contains(c))
        .take(NAME_MAX)
        .collect()
}

/// A typed name as it is saved.
pub(crate) fn typed_name(name: &str) -> String {
    clean_name(name).trim().to_string()
}

/// The Console's `GlobalSettingsDraft`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub startup_mode: u8,
    pub default_slot: u8,
    pub master_volume_mode: u8,
    pub output_config_mode: u8,
    pub dac: DacHwMuteConfig,
    /// `OutputLatencyId`; 0, never a valid id, when the device has no setting.
    pub output_latency: u8,
}

impl Draft {
    pub fn from_device(state: &DeviceState, data: &SettingsData) -> Self {
        let d = state.dac_hw_mute();
        Self {
            startup_mode: data.startup_mode(),
            default_slot: data.default_slot(),
            master_volume_mode: data.master_volume_mode(),
            output_config_mode: data.output_config_mode(),
            dac: DacHwMuteConfig {
                enabled: d.enabled,
                active_low: d.active_low,
                pin: d.pin,
                hold_ms: d.hold_ms,
                release_ms: d.release_ms,
            },
            output_latency: data.extras.latency.as_ref().map_or(0, |l| l.requested_id),
        }
    }
}

const STARTUP_MODES: [&str; 2] = ["Specified Default", "Last Used"];
const PERSIST_MODES: [&str; 2] = ["Independent", "With Preset"];
const POLARITY: [&str; 2] = ["Active Low", "Active High"];
const HOLD_MS: [u16; 5] = [5, 10, 20, 50, 100];
const RELEASE_MS: [u16; 6] = [0, 5, 10, 20, 50, 100];

/// The offered times, plus whatever the device is actually holding.
///
/// The list is the Console's `globalMsPicker` menu; a device carrying a value
/// that is not on it (7 ms, say) gets that value appended rather than being
/// misreported as the first entry.
fn ms_values(list: &[u16], current: u16) -> Vec<u16> {
    let mut out = list.to_vec();
    if !out.contains(&current) {
        out.push(current);
    }
    out
}

fn ms_choices(list: &[u16], current: u16) -> (Vec<String>, usize) {
    let values = ms_values(list, current);
    let at = values.iter().position(|m| *m == current).unwrap_or(0);
    (values.iter().map(|m| format!("{m} ms")).collect(), at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    DeviceName,
    StartupMode,
    DefaultPreset,
    MuteEnable,
    Polarity,
    MutePin,
    Hold,
    Release,
    Test,
    Latency,
    MasterMode,
    HardwareMode,
}

pub struct GlobalPage {
    cursor: usize,
    draft: Draft,
    /// The draft as it last matched the device, for Revert.
    clean: Draft,
    /// True once the person has changed something, which is what stops fresh
    /// device data from looking unsaved (the Console's `globalUserEdited`).
    edited: bool,
    /// Which picker opened the popup that is up.
    popup_item: Option<Item>,
    /// The device-name dialog is up.
    naming: bool,
    /// The device-name write's answer, shown under the row.
    status: Option<(String, bool)>,
}

const KEYS: &[KeyHelp] = &[
    KeyHelp::new("↑ ↓", "Move between controls"),
    KeyHelp::new("← →", "Change"),
    KeyHelp::new("Enter", "Open the list, edit a name, or start the test"),
    KeyHelp::new("Space", "Toggle"),
    KeyHelp::new("Tab", "Sidebar, page, save bar"),
    KeyHelp::new("[ ]", "Back, forward"),
    KeyHelp::new("Esc", "Leave Settings"),
];

impl GlobalPage {
    pub fn new(state: &DeviceState, data: &SettingsData) -> Self {
        let draft = Draft::from_device(state, data);
        Self {
            cursor: 0,
            clean: draft.clone(),
            draft,
            edited: false,
            popup_item: None,
            naming: false,
            status: None,
        }
    }

    /// Whether the staged draft differs from what the device holds.
    pub fn dirty(&self, state: &DeviceState, data: &SettingsData) -> bool {
        self.edited && self.draft != Draft::from_device(state, data)
    }

    /// Take the draft as saved; the next `dirty` reads clean.
    pub fn mark_saved(&mut self) {
        self.edited = false;
        self.clean = self.draft.clone();
    }

    // -- WP TM19 --
    /// Take as saved the fields these written lines carried (the commands of
    /// `save_commands`). Any other edit stays in the draft, and Revert goes
    /// back to the fields as they are on the device now.
    pub fn mark_written(&mut self, lines: &[String]) {
        for line in lines {
            match line.split_whitespace().next() {
                Some("preset.startup") => {
                    self.clean.startup_mode = self.draft.startup_mode;
                    self.clean.default_slot = self.draft.default_slot;
                }
                Some("vol.master.mode") => {
                    self.clean.master_volume_mode = self.draft.master_volume_mode;
                }
                Some("preset.iomode") => {
                    self.clean.output_config_mode = self.draft.output_config_mode;
                }
                Some("dev.dacmute") => self.clean.dac = self.draft.dac.clone(),
                Some("out.latency") => self.clean.output_latency = self.draft.output_latency,
                _ => {}
            }
        }
        self.edited = self.draft != self.clean;
    }

    pub fn revert(&mut self) {
        self.edited = false;
        self.draft = self.clean.clone();
    }

    /// The writes Save issues, in the Console's order and only for what
    /// actually changed.
    pub fn save_commands(&self, state: &DeviceState, data: &SettingsData) -> Vec<String> {
        let device = Draft::from_device(state, data);
        let d = &self.draft;
        let mut out = Vec::new();
        if d.startup_mode != device.startup_mode || d.default_slot != device.default_slot {
            out.push(format!(
                "preset.startup mode={} slot={}",
                if d.startup_mode == 1 {
                    "last_active"
                } else {
                    "specified"
                },
                d.default_slot
            ));
        }
        if d.master_volume_mode != device.master_volume_mode {
            out.push(format!(
                "vol.master.mode {}",
                if d.master_volume_mode == 1 {
                    "with-preset"
                } else {
                    "independent"
                }
            ));
        }
        if d.output_config_mode != device.output_config_mode {
            out.push(format!(
                "preset.iomode {}",
                if d.output_config_mode == 1 {
                    "with-preset"
                } else {
                    "independent"
                }
            ));
        }
        if d.dac != device.dac {
            out.push(format!(
                "dev.dacmute enabled={} active_low={} pin={} hold_ms={} release_ms={}",
                if d.dac.enabled { "on" } else { "off" },
                if d.dac.active_low { "on" } else { "off" },
                d.dac.pin,
                d.dac.hold_ms,
                d.dac.release_ms
            ));
        }
        if d.output_latency != device.output_latency && d.output_latency != 0 {
            out.push(format!("out.latency {}", d.output_latency));
        }
        out
    }

    /// `n: <name|Empty>`, the Console's `slotLabel`.
    fn slot_label(data: &SettingsData, slot: u8) -> String {
        let name = if data.occupied(slot) {
            let n = data
                .preset_names
                .get(slot as usize)
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if n.is_empty() {
                format!("Preset {}", slot + 1)
            } else {
                n
            }
        } else {
            "Empty".to_string()
        };
        format!("{}: {name}", slot + 1)
    }

    fn pin_choices(&self, cx: &Cx<'_>) -> Vec<u8> {
        cx.free_pins("DAC Mute", Some(self.draft.dac.pin))
    }

    fn build(&self, cx: &Cx<'_>) -> Vec<(Option<Item>, Row)> {
        let d = &self.draft;
        let mut rows: Vec<(Option<Item>, Row)> = Vec::new();
        if let Some(name) = &cx.data.extras.device_name {
            rows.push((None, Row::section("Device Name")));
            rows.push((
                Some(Item::DeviceName),
                Row::Buttons {
                    label: "Name".into(),
                    caption: Some(format!("Serial {}", cx.state.caps.serial)),
                    buttons: vec![if name.is_empty() {
                        "Add a name".to_string()
                    } else {
                        truncate(name, NAME_BUTTON_WIDTH)
                    }],
                    cursor: 0,
                    enabled: cx.connected,
                },
            ));
            if let Some((text, err)) = &self.status {
                rows.push((None, Row::Status(text.clone(), *err)));
            }
            rows.push((
                None,
                Row::note(
                    "Stored on the device, so it keeps this name on any computer or DSPi Link \
                     hub. Shown in the device picker and on the front panel.",
                ),
            ));
            rows.push((None, Row::Blank));
        }
        rows.push((None, Row::section("Startup Preset")));
        rows.push((
            Some(Item::StartupMode),
            Row::Pick {
                label: "Mode".into(),
                choices: STARTUP_MODES.iter().map(|s| (*s).to_string()).collect(),
                selected: d.startup_mode.min(1) as usize,
                caption: None,
                enabled: true,
            },
        ));
        if d.startup_mode == 0 {
            rows.push((
                Some(Item::DefaultPreset),
                Row::Pick {
                    label: "Default Preset".into(),
                    choices: (0..10).map(|s| Self::slot_label(cx.data, s)).collect(),
                    selected: d.default_slot.min(9) as usize,
                    caption: None,
                    enabled: true,
                },
            ));
        }
        rows.push((
            None,
            Row::note("Choose which preset loads when the device powers on."),
        ));

        if cx.feature("dac_hardware_mute") {
            rows.push((None, Row::Blank));
            rows.push((None, Row::section("External Mute Control")));
            rows.push((
                None,
                Row::warning(
                    "Adjust only with audio stopped",
                    "Changing these settings while audio is playing can send a loud pop or \
                     full-level transient to your amplifier and speakers. Stop playback before \
                     making changes.",
                ),
            ));
            rows.push((
                Some(Item::MuteEnable),
                Row::Toggle {
                    label: "Enable Automatic Mute".into(),
                    on: d.dac.enabled,
                    caption: Some(
                        "Briefly mute an external DAC or amplifier to suppress loud pops during \
                         system state changes."
                            .into(),
                    ),
                    enabled: true,
                },
            ));
            if d.dac.enabled {
                rows.push((
                    Some(Item::Polarity),
                    Row::Pick {
                        label: "Polarity".into(),
                        choices: POLARITY.iter().map(|s| (*s).to_string()).collect(),
                        selected: usize::from(!d.dac.active_low),
                        caption: None,
                        enabled: true,
                    },
                ));
                let pins = self.pin_choices(cx);
                rows.push((
                    Some(Item::MutePin),
                    Row::Pick {
                        label: "Mute Pin".into(),
                        choices: pins.iter().map(|p| format!("GPIO {p}")).collect(),
                        selected: pins.iter().position(|p| *p == d.dac.pin).unwrap_or(0),
                        caption: None,
                        enabled: !pins.is_empty(),
                    },
                ));
                let (hold, hold_at) = ms_choices(&HOLD_MS, d.dac.hold_ms);
                rows.push((
                    Some(Item::Hold),
                    Row::Pick {
                        label: "Hold Time".into(),
                        choices: hold,
                        selected: hold_at,
                        caption: None,
                        enabled: true,
                    },
                ));
                let (release, release_at) = ms_choices(&RELEASE_MS, d.dac.release_ms);
                rows.push((
                    Some(Item::Release),
                    Row::Pick {
                        label: "Release Time".into(),
                        choices: release,
                        selected: release_at,
                        caption: None,
                        enabled: true,
                    },
                ));
                // The Console tests the device's live configuration, not the
                // draft, so a staged change has to be saved first.
                let live = cx.state.dac_hw_mute();
                let can_test = cx.connected
                    && live.enabled
                    && live.pin != DacHwMuteConfig::PIN_NONE
                    && !cx.global_dirty;
                rows.push((
                    Some(Item::Test),
                    Row::Buttons {
                        label: "Test".into(),
                        caption: Some(if cx.global_dirty {
                            "Save your changes first to test the mute pin.".into()
                        } else {
                            "Toggle automatic mute for one second to confirm hardware \
                             configuration."
                                .to_string()
                        }),
                        buttons: vec!["Start".into()],
                        cursor: 0,
                        enabled: can_test,
                    },
                ));
            }
        }

        if let Some(latency) = &cx.data.extras.latency
            && !latency.options.is_empty()
        {
            rows.push((None, Row::Blank));
            rows.push((None, Row::section("Latency")));
            let (choices, selected) = latency_choices(&latency.options, d.output_latency);
            rows.push((
                Some(Item::Latency),
                Row::Pick {
                    label: "Target".into(),
                    choices,
                    selected,
                    caption: None,
                    enabled: true,
                },
            ));
            rows.push((
                None,
                Row::note(latency_caption(&latency.options, d.output_latency)),
            ));
        }

        rows.push((None, Row::Blank));
        rows.push((None, Row::section("Master Volume")));
        rows.push((
            Some(Item::MasterMode),
            Row::Pick {
                label: "Mode".into(),
                choices: PERSIST_MODES.iter().map(|s| (*s).to_string()).collect(),
                selected: d.master_volume_mode.min(1) as usize,
                caption: None,
                enabled: true,
            },
        ));
        rows.push((
            None,
            Row::note(if d.master_volume_mode == 0 {
                "Master volume is stored on the device independently of presets and applied at \
                 boot. Loading a preset never changes it."
            } else {
                "Master volume is part of each preset. Saved with the preset, restored on preset \
                 load."
            }),
        ));

        rows.push((None, Row::Blank));
        rows.push((None, Row::section("Hardware Configuration")));
        rows.push((
            Some(Item::HardwareMode),
            Row::Pick {
                label: "Mode".into(),
                choices: PERSIST_MODES.iter().map(|s| (*s).to_string()).collect(),
                selected: d.output_config_mode.min(1) as usize,
                caption: None,
                enabled: true,
            },
        ));
        rows.push((
            None,
            Row::note(if d.output_config_mode == 0 {
                "Input and output configuration is stored on the device independently and applied \
                 at boot. Loading a preset never changes your wiring."
            } else {
                "Input and output configuration is part of each preset. Saved with the preset and \
                 restored on preset load."
            }),
        ));
        rows
    }

    fn item(&self, index: usize, cx: &Cx<'_>) -> Option<Item> {
        self.build(cx)
            .into_iter()
            .filter(|(_, r)| r.focusable())
            .nth(index)
            .and_then(|(i, _)| i)
    }

    fn apply(&mut self, item: Item, choice: usize, cx: &Cx<'_>) -> PageEvent {
        let d = &mut self.draft;
        match item {
            Item::StartupMode => d.startup_mode = choice.min(1) as u8,
            Item::DefaultPreset => d.default_slot = choice.min(9) as u8,
            Item::Polarity => d.active_low_from(choice),
            Item::MutePin => {
                let pins = cx.free_pins("DAC Mute", Some(d.dac.pin));
                if let Some(p) = pins.get(choice) {
                    d.dac.pin = *p;
                }
            }
            Item::Hold => {
                if let Some(m) = ms_values(&HOLD_MS, d.dac.hold_ms).get(choice) {
                    d.dac.hold_ms = *m;
                }
            }
            Item::Release => {
                if let Some(m) = ms_values(&RELEASE_MS, d.dac.release_ms).get(choice) {
                    d.dac.release_ms = *m;
                }
            }
            Item::Latency => {
                match cx
                    .data
                    .extras
                    .latency
                    .as_ref()
                    .and_then(|l| l.options.get(choice))
                {
                    Some(option) => d.output_latency = option.id,
                    // The "-" entry an id off the table adds: not a choice.
                    None => return PageEvent::Handled,
                }
            }
            Item::MasterMode => d.master_volume_mode = choice.min(1) as u8,
            Item::HardwareMode => d.output_config_mode = choice.min(1) as u8,
            _ => return PageEvent::Handled,
        }
        self.edited = true;
        PageEvent::Handled
    }
}

/// The picker's entries and which one is `id`: `-` when the device holds an id
/// its table does not list, as the Console draws it.
fn latency_choices(options: &[LatencyOption], id: u8) -> (Vec<String>, usize) {
    let mut choices: Vec<String> = options
        .iter()
        .map(|o| {
            let ms = latency_ms(o.latency_us);
            if o.id == LATENCY_DEFAULT_ID {
                format!("{ms} (default)")
            } else {
                ms
            }
        })
        .collect();
    let selected = match options.iter().position(|o| o.id == id) {
        Some(at) => at,
        None => {
            choices.push("-".to_string());
            choices.len().saturating_sub(1)
        }
    };
    (choices, selected)
}

/// The Console's `latencyCaption`: the trade-off, and what the selected option
/// really runs as when the rate caps it.
fn latency_caption(options: &[LatencyOption], id: u8) -> String {
    let mut text = "Lower values reduce the delay from input to output. Higher values may benefit \
                    sources exhibiting high jitter in the event of audio stutters."
        .to_string();
    if let Some(o) = options.iter().find(|o| o.id == id)
        && o.available
        && o.effective_us != o.latency_us
    {
        text.push_str(&format!(
            " At the current sample rate the output buffer is smaller than this, so it runs as \
             {}.",
            latency_ms(o.effective_us)
        ));
    }
    text.push_str(" Values less than 2ms have no effect upon USB Audio.");
    text
}

impl Draft {
    fn active_low_from(&mut self, choice: usize) {
        self.dac.active_low = choice == 0;
    }
}

impl SettingsPage for GlobalPage {
    fn rows(&self, cx: &Cx<'_>) -> Vec<Row> {
        self.build(cx).into_iter().map(|(_, r)| r).collect()
    }

    fn act(&mut self, index: usize, action: Action, cx: &Cx<'_>) -> PageEvent {
        let Some(item) = self.item(index, cx) else {
            return PageEvent::Handled;
        };
        match (item, action) {
            (Item::MuteEnable, Action::Toggled(on)) => {
                self.draft.dac.enabled = on;
                // Enabling with no pin is nonsensical, so the first free GPIO
                // is taken, as the Console does.
                if on
                    && self.draft.dac.pin == DacHwMuteConfig::PIN_NONE
                    && let Some(p) = cx.free_pins("DAC Mute", None).first()
                {
                    self.draft.dac.pin = *p;
                }
                self.edited = true;
                PageEvent::Handled
            }
            (Item::Test, Action::Open) => PageEvent::Command("dev.dacmute.test".into()),
            (Item::DeviceName, Action::Open) => {
                self.naming = true;
                self.status = None;
                PageEvent::Dialog(Dialog::text(
                    "Device Name",
                    "Up to 31 plain characters. Saving writes it to the device at once, with a \
                     brief output restart. An empty name removes it.",
                    cx.data.extras.device_name.clone().unwrap_or_default(),
                    "Add a name",
                ))
            }
            (_, Action::Selected(i)) => self.apply(item, i, cx),
            (_, Action::Open) => {
                // A picker with more than a handful of entries gets the list.
                let rows = self.build(cx);
                let row = rows
                    .iter()
                    .filter(|(_, r)| r.focusable())
                    .nth(index)
                    .map(|(_, r)| r);
                if let Some(Row::Pick {
                    label,
                    choices,
                    selected,
                    ..
                }) = row
                {
                    self.popup_item = Some(item);
                    return PageEvent::Popup(PopupList::new(
                        label.clone(),
                        choices.clone(),
                        *selected,
                    ));
                }
                PageEvent::Handled
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
        KEYS
    }

    fn popup_result(&mut self, choice: Option<usize>, cx: &Cx<'_>) -> PageEvent {
        let Some(item) = self.popup_item.take() else {
            return PageEvent::Handled;
        };
        match choice {
            Some(i) => self.apply(item, i, cx),
            None => PageEvent::Handled,
        }
    }

    fn dialog_result(&mut self, outcome: DialogOutcome, cx: &Cx<'_>) -> PageEvent {
        if !std::mem::take(&mut self.naming) {
            return PageEvent::Handled;
        }
        let DialogOutcome::Text(typed) = outcome else {
            return PageEvent::Handled;
        };
        let name = typed_name(&typed);
        if cx.data.extras.device_name.as_deref() == Some(name.as_str()) {
            return PageEvent::Handled;
        }
        let done = if name.is_empty() {
            "Device name removed".to_string()
        } else {
            format!("Device name set to {name}")
        };
        PageEvent::Session(super::device_write(
            0,
            "dev.name",
            Vec::new(),
            Value::Text(name),
            Rc::new(move |code| {
                if code == st::SUCCESS {
                    (done.clone(), false)
                } else {
                    ("Failed to set the device name".to_string(), true)
                }
            }),
        ))
    }

    fn session_result(&mut self, _tag: u32, reply: SessionReply, _cx: &Cx<'_>) -> PageEvent {
        self.status = match reply {
            SessionReply::Ok(m) => Some((m, false)),
            SessionReply::Err(m) => Some((m, true)),
            SessionReply::Bytes(_) => None,
        };
        match &self.status {
            Some((m, _)) => PageEvent::Status(m.clone()),
            None => PageEvent::Handled,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    /// The Console appends an out-of-list value rather than snapping to the
    /// head of the menu: a device holding 7 ms must not read as 5 ms.
    #[test]
    fn an_off_list_hold_time_is_offered_as_itself() {
        let (choices, at) = super::ms_choices(&super::HOLD_MS, 7);
        assert_eq!(choices.last().map(String::as_str), Some("7 ms"));
        assert_eq!(at, choices.len() - 1);
        assert_eq!(super::ms_values(&super::HOLD_MS, 7)[at], 7);

        // A value that is on the list is not duplicated.
        let (choices, at) = super::ms_choices(&super::HOLD_MS, 20);
        assert_eq!(choices.len(), super::HOLD_MS.len());
        assert_eq!(choices[at], "20 ms");

        // Release keeps its zero, which is a real setting and not a fallback.
        let (choices, at) = super::ms_choices(&super::RELEASE_MS, 0);
        assert_eq!(choices[at], "0 ms");
    }

    use super::super::Page;
    use super::super::tests::{data, frame, key, screen, state};
    use super::*;
    use crate::shell::{Screen, ScreenEvent};
    use crossterm::event::KeyCode;

    #[test]
    fn the_page_carries_every_section_in_the_consoles_order() {
        let (mut s, st) = screen(Page::Global);
        for (w, h) in [(120u16, 40u16), (80, 24)] {
            let f = frame(&mut s, &st, w, h);
            assert_eq!(f.lines().count(), h as usize);
            assert!(f.contains("STARTUP PRESET"), "{w}x{h}: {f}");
            assert!(f.contains("Specified Default"), "{w}x{h}: {f}");
        }
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("3: Living Room"), "the default preset: {f}");
        assert!(
            f.contains("Choose which preset loads when the device powers on."),
            "{f}"
        );
        assert!(f.contains("EXTERNAL MUTE CONTROL"), "{f}");
        assert!(f.contains("▲ Adjust only with audio stopped"), "{f}");
        assert!(f.contains("Enable Automatic Mute"), "{f}");
    }

    #[test]
    fn the_lower_sections_carry_their_captions() {
        let (mut s, st) = screen(Page::Global);
        s.handle(key(KeyCode::Tab), &st);
        for _ in 0..10 {
            s.handle(key(KeyCode::Down), &st);
        }
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("MASTER VOLUME"), "{f}");
        assert!(f.contains("HARDWARE CONFIGURATION"), "{f}");
        assert!(f.contains("Independent"), "{f}");
        assert!(
            f.contains("Loading a preset never changes your wiring."),
            "{f}"
        );
    }

    #[test]
    fn the_draft_is_staged_and_only_written_on_save() {
        let st = state();
        let d = data();
        let mut p = GlobalPage::new(&st, &d);
        assert!(!p.dirty(&st, &d));
        let cx = Cx {
            state: &st,
            data: &d,
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        // Startup mode is the first focusable row.
        assert_eq!(p.act(0, Action::Selected(1), &cx), PageEvent::Handled);
        assert!(p.dirty(&st, &d));
        let cmds = p.save_commands(&st, &d);
        assert_eq!(cmds, vec!["preset.startup mode=last_active slot=2"]);
        p.mark_saved();
        assert!(!p.dirty(&st, &d));
    }

    #[test]
    fn every_changed_field_gets_its_own_write() {
        let st = state();
        let d = data();
        let mut p = GlobalPage::new(&st, &d);
        p.draft.startup_mode = 1;
        p.draft.master_volume_mode = 1;
        p.draft.output_config_mode = 1;
        p.draft.dac.hold_ms = 20;
        p.edited = true;
        let cmds = p.save_commands(&st, &d);
        assert_eq!(
            cmds,
            vec![
                "preset.startup mode=last_active slot=2",
                "vol.master.mode with-preset",
                "preset.iomode with-preset",
                "dev.dacmute enabled=on active_low=on pin=11 hold_ms=20 release_ms=0",
            ]
        );
    }

    #[test]
    fn the_mute_test_waits_for_the_draft_to_be_saved() {
        let st = state();
        let d = data();
        let mut p = GlobalPage::new(&st, &d);
        let dirty_cx = Cx {
            state: &st,
            data: &d,
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: true,
        };
        let rows = p.build(&dirty_cx);
        let test = rows
            .iter()
            .find(|(i, _)| *i == Some(Item::Test))
            .map(|(_, r)| r)
            .expect("the test row");
        match test {
            Row::Buttons {
                enabled, caption, ..
            } => {
                assert!(!enabled);
                assert_eq!(
                    caption.as_deref(),
                    Some("Save your changes first to test the mute pin.")
                );
            }
            other => panic!("{other:?}"),
        }
        // Clean, it fires the one-second pulse.
        let clean = Cx {
            state: &st,
            data: &d,
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        let index = p
            .build(&clean)
            .iter()
            .filter(|(_, r)| r.focusable())
            .position(|(i, _)| *i == Some(Item::Test))
            .expect("the test row is focusable");
        assert_eq!(
            p.act(index, Action::Open, &clean),
            PageEvent::Command("dev.dacmute.test".into())
        );
    }

    #[test]
    fn enabling_the_mute_takes_the_first_free_pin() {
        let st = state();
        let d = data();
        let mut p = GlobalPage::new(&st, &d);
        p.draft.dac.enabled = false;
        p.draft.dac.pin = DacHwMuteConfig::PIN_NONE;
        let cx = Cx {
            state: &st,
            data: &d,
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        let index = p
            .build(&cx)
            .iter()
            .filter(|(_, r)| r.focusable())
            .position(|(i, _)| *i == Some(Item::MuteEnable))
            .expect("the enable row");
        p.act(index, Action::Toggled(true), &cx);
        assert_ne!(p.draft.dac.pin, DacHwMuteConfig::PIN_NONE);
        assert!(
            !claimed(&st, &d, p.draft.dac.pin),
            "it took a pin nothing else holds"
        );
    }

    fn claimed(st: &DeviceState, d: &SettingsData, pin: u8) -> bool {
        d.pin_claims(st).iter().any(|c| c.gpio == pin)
    }

    #[test]
    fn the_pin_picker_marks_a_claimed_pin_by_leaving_it_out() {
        let st = state();
        let d = data();
        let p = GlobalPage::new(&st, &d);
        let cx = Cx {
            state: &st,
            data: &d,
            config: &super::super::AppConfig::default(),
            connected: true,
            global_dirty: false,
        };
        let pins = p.pin_choices(&cx);
        assert!(pins.contains(&11), "its own pin is always offered");
        assert!(!pins.contains(&6), "GPIO 6 is Output 1");
        assert!(!pins.contains(&14), "GPIO 14 is the I2S BCK");
    }

    #[test]
    fn enter_on_a_picker_opens_its_list() {
        let (mut s, st) = screen(Page::Global);
        s.handle(key(KeyCode::Tab), &st);
        s.handle(key(KeyCode::Down), &st);
        match s.handle(key(KeyCode::Enter), &st) {
            ScreenEvent::Popup(p) => {
                assert_eq!(p.title, "Default Preset");
                assert_eq!(p.items[0], "1: Movies");
                assert_eq!(p.items[3], "4: Empty");
            }
            other => panic!("{other:?}"),
        }
        s.popup_result(Some(5), &st);
        assert!(s.dirty(&st));
    }

    // ---- WP TM17: device name and output latency -------------------------

    use crate::settings::{AppConfig, SettingsScreen};
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::{FakeDevice, Transport};

    /// A session on the demo device, with what the page sees of it. `keep`
    /// names which of the three features the probe is said to have found.
    pub(in crate::settings) fn demo(keep: &[&str]) -> (Session, DeviceState, SettingsScreen) {
        let mut t: Box<dyn Transport> = Box::new(FakeDevice::demo());
        let mut caps = dspi_session::probe(t.as_mut()).expect("probe");
        caps.features.retain(|f| {
            !["device_name", "input_source_names", "output_latency"].contains(&f.name.as_str())
                || keep.contains(&f.name.as_str())
        });
        let mut session = Session::new(t, caps.clone()).expect("session");
        let bulk = session.snapshot().expect("bulk");
        let st = DeviceState::new(caps, bulk);
        let d = SettingsData::read(&mut session);
        let screen = SettingsScreen::new(&st, d, AppConfig::default())
            .config_path(crate::settings::tests::scratch("tm17"));
        (session, st, screen)
    }

    pub(in crate::settings) const ALL: [&str; 3] =
        ["device_name", "input_source_names", "output_latency"];

    /// Open the dialog on the current row, answer it with `typed`, run the
    /// write it makes and hand the reply back, as the shell does.
    pub(in crate::settings) fn answer(
        s: &mut SettingsScreen,
        st: &DeviceState,
        session: &mut Session,
        typed: &str,
    ) {
        assert!(
            matches!(s.handle(key(KeyCode::Enter), st), ScreenEvent::Dialog(_)),
            "the row opens a dialog"
        );
        match s.dialog_result(DialogOutcome::Text(typed.into()), st) {
            ScreenEvent::Session(req) => {
                let reply = (req.run)(session);
                s.session_result(req.tag, reply, st);
            }
            other => panic!("{other:?}"),
        }
    }

    fn device_name(session: &mut Session) -> Vec<u8> {
        session
            .with_transport(|t| t.control_in(op::REQ_GET_DEVICE_NAME, 0, 32))
            .expect("read")
    }

    /// A device with all three, for a test without a session.
    pub(in crate::settings) fn extras() -> DeviceExtras {
        let option = |id, latency_us: u16| LatencyOption {
            id,
            available: true,
            latency_us,
            effective_us: latency_us,
        };
        let mut source_names = vec![Some(String::new()); usize::from(SOURCE_COUNT)];
        source_names[1] = Some("Living Room TV".to_string());
        DeviceExtras {
            device_name: Some("DSPi Demo".to_string()),
            source_names,
            latency: Some(Latency {
                requested_id: LATENCY_DEFAULT_ID,
                options: [667, 1000, 2000, 3000, 4000, 6000, 8000]
                    .into_iter()
                    .zip(1..)
                    .map(|(us, id)| option(id, us))
                    .collect(),
            }),
        }
    }

    fn wp_tm17_cx<'a>(st: &'a DeviceState, d: &'a SettingsData, c: &'a AppConfig) -> Cx<'a> {
        Cx {
            state: st,
            data: d,
            config: c,
            connected: true,
            global_dirty: false,
        }
    }

    #[test]
    fn wp_tm17_the_device_name_round_trips_on_the_demo_device() {
        let (mut session, st, s) = demo(&ALL);
        let mut s = s.open(Page::Global, &st);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("DEVICE NAME"), "{f}");
        assert!(f.contains("DSPi Demo"), "{f}");
        assert!(f.contains(&format!("Serial {}", st.caps.serial)), "{f}");

        s.handle(key(KeyCode::Tab), &st);
        // A space and a quote, which the command grammar could not carry.
        answer(&mut s, &st, &mut session, "  Rack \"left\" ");
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("Rack \"left\""), "{f}");
        assert!(f.contains("Device name set to"), "{f}");
        assert!(!s.dirty(&st), "the dialog's OK was the commit");
        assert!(!s.cs_dirty(), "not a control-surface edit");
        assert!(device_name(&mut session).starts_with(b"Rack \"left\"\0"));

        // What a re-read of the device finds is the same.
        assert_eq!(
            DeviceExtras::read(&mut session).device_name.as_deref(),
            Some("Rack \"left\"")
        );

        // An empty name removes it.
        answer(&mut s, &st, &mut session, "");
        assert_eq!(device_name(&mut session), vec![0; 32]);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("Add a name"), "{f}");
        assert!(f.contains("Device name removed"), "{f}");
    }

    #[test]
    fn wp_tm17_an_unchanged_or_cancelled_name_writes_nothing() {
        let (_session, st, s) = demo(&ALL);
        let mut s = s.open(Page::Global, &st);
        s.handle(key(KeyCode::Tab), &st);
        s.handle(key(KeyCode::Enter), &st);
        assert_eq!(
            s.dialog_result(DialogOutcome::Text(" DSPi Demo ".into()), &st),
            ScreenEvent::Handled
        );
        s.handle(key(KeyCode::Enter), &st);
        assert_eq!(
            s.dialog_result(DialogOutcome::Cancelled, &st),
            ScreenEvent::Handled
        );
    }

    #[test]
    fn wp_tm17_a_name_from_elsewhere_shows_on_the_next_poll() {
        let (mut session, st, s) = demo(&ALL);
        let mut s = s.open(Page::Global, &st);
        session
            .write("dev.name", &[], Value::Text("Moved".into()))
            .expect("write");
        s.poll(&mut session, &st);
        let f = frame(&mut s, &st, 120, 40);
        assert!(f.contains("Moved"), "{f}");
    }

    #[test]
    fn wp_tm17_the_latency_is_staged_and_round_trips_on_save() {
        let (mut session, st, s) = demo(&ALL);
        let mut s = s.open(Page::Global, &st);
        let f = frame(&mut s, &st, 120, 80);
        assert!(f.contains("LATENCY"), "{f}");
        assert!(f.contains("4 ms (default)"), "{f}");
        assert!(f.contains("no effect upon USB Audio."), "{f}");

        let requested = |session: &mut Session| {
            session
                .with_transport(|t| t.control_in(op::REQ_OUTPUT_LATENCY, 0, 12))
                .expect("status")[0]
        };
        assert_eq!(requested(&mut session), LATENCY_DEFAULT_ID);

        s.handle(key(KeyCode::Tab), &st);
        let at = s
            .pages
            .global
            .build(&wp_tm17_cx(&st, &s.data, &s.config))
            .iter()
            .filter(|(_, r)| r.focusable())
            .position(|(i, _)| *i == Some(Item::Latency))
            .expect("the latency row");
        for _ in 0..at {
            s.handle(key(KeyCode::Down), &st);
        }
        // One to the left of 4 ms is 3 ms: staged, nothing on the wire yet.
        assert_eq!(s.handle(key(KeyCode::Left), &st), ScreenEvent::Handled);
        assert!(s.dirty(&st));
        assert_eq!(requested(&mut session), LATENCY_DEFAULT_ID);

        let line = match s.save(&st) {
            ScreenEvent::Command(c) => c,
            other => panic!("{other:?}"),
        };
        assert_eq!(line, "out.latency 4");

        // Through the grammar the interface parses it with, and the same write.
        let tokens = dspi_cmd::tokenize(&line);
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        match dspi_cmd::parse(&refs, &dspi_cmd::Context::default()).expect("parse") {
            dspi_cmd::Command::Set {
                path,
                indices,
                value,
            } => {
                let out = session.write(path, &indices, value).expect("write");
                assert!(out.is_confirmed(), "{out:?}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(requested(&mut session), 4);
        s.poll(&mut session, &st);
        assert!(!s.dirty(&st));
        let f = frame(&mut s, &st, 120, 80);
        assert!(f.contains("3 ms"), "{f}");

        // A fresh read, as the next Settings open makes, agrees.
        let fresh = SettingsData::read(&mut session);
        assert_eq!(fresh.extras.latency.map(|l| l.requested_id), Some(4));
    }

    /// Revert puts the picker back on what the device holds.
    #[test]
    fn wp_tm17_revert_drops_a_staged_latency() {
        let st = state();
        let mut d = data();
        d.extras = extras();
        let mut p = GlobalPage::new(&st, &d);
        p.draft.output_latency = 1;
        p.edited = true;
        assert!(p.dirty(&st, &d));
        assert_eq!(p.save_commands(&st, &d), vec!["out.latency 1"]);
        p.revert();
        assert_eq!(p.draft.output_latency, LATENCY_DEFAULT_ID);
        assert!(!p.dirty(&st, &d));
    }

    #[test]
    fn wp_tm17_rows_are_hidden_when_the_device_lacks_them() {
        let (_session, st, s) = demo(&[]);
        assert_eq!(s.data.extras, DeviceExtras::default(), "nothing was read");
        let mut s = s.open(Page::Global, &st);
        let f = frame(&mut s, &st, 120, 80);
        assert!(!f.contains("DEVICE NAME"), "{f}");
        assert!(!f.contains("LATENCY"), "{f}");
        assert!(f.contains("STARTUP PRESET"), "{f}");

        // Each goes on its own.
        let (_session, st, s) = demo(&["output_latency"]);
        let mut s = s.open(Page::Global, &st);
        let f = frame(&mut s, &st, 120, 80);
        assert!(!f.contains("DEVICE NAME"), "{f}");
        assert!(f.contains("LATENCY"), "{f}");

        // An empty option table is no picker, as on the Console.
        let st = state();
        let mut d = data();
        d.extras = extras();
        d.extras.latency.as_mut().expect("latency").options.clear();
        let mut s = SettingsScreen::new(&st, d, AppConfig::default()).open(Page::Global, &st);
        let f = frame(&mut s, &st, 120, 80);
        assert!(!f.contains("LATENCY"), "{f}");

        // A failed save never targets id 0, the "no setting" value.
        let st = state();
        let d = data();
        let mut p = GlobalPage::new(&st, &d);
        p.edited = true;
        assert!(p.save_commands(&st, &d).is_empty());
    }

    /// The caption's middle sentence and the off-table entry, both
    /// the device's word rather than anything compiled in.
    #[test]
    fn wp_tm17_the_picker_and_caption_follow_the_device() {
        let o = |id, available, latency_us, effective_us| LatencyOption {
            id,
            available,
            latency_us,
            effective_us,
        };
        let options = [o(1, true, 667, 667), o(5, true, 4000, 3660)];
        let (choices, at) = latency_choices(&options, 5);
        assert_eq!(choices, ["0.67 ms", "4 ms (default)"]);
        assert_eq!(at, 1);
        assert!(latency_caption(&options, 5).contains("so it runs as 3.66 ms."));
        assert!(!latency_caption(&options, 1).contains("runs as"));

        let (choices, at) = latency_choices(&options, 9);
        assert_eq!(choices.last().map(String::as_str), Some("-"));
        assert_eq!(at, 2);

        // Picking the "-" entry is not an edit.
        let st = state();
        let d = data();
        let c = AppConfig::default();
        let cx = wp_tm17_cx(&st, &d, &c);
        let mut p = GlobalPage::new(&st, &d);
        p.apply(Item::Latency, 99, &cx);
        assert!(!p.edited);
    }

    #[test]
    fn wp_tm17_device_bytes_are_decoded_defensively() {
        assert_eq!(decode_latency_status(&[5; 11]), None);
        let mut status = [0u8; 12];
        status[0] = 3;
        status[9] = 7;
        assert_eq!(decode_latency_status(&status), Some((3, 7)));
        // A trailing partial entry is dropped.
        let table = [2, 0x80, 0xE8, 0x03, 0, 0, 0xE8, 0x03, 9, 9];
        let got = decode_latency_options(&table);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, 2);
        assert!(got[0].available);
        assert_eq!(got[0].latency_us, 1000);

        // No escape reaches the terminal, and a long name stops at 31.
        assert_eq!(clean_name("Dev\u{1b}[2J\u{e9}"), "Dev[2J");
        assert_eq!(clean_name(&"x".repeat(40)).len(), 31);
        assert_eq!(typed_name("  a b  "), "a b");

        // A full-length name fits a narrow page without touching the label.
        let st = state();
        let mut d = data();
        d.extras.device_name = Some("x".repeat(31));
        let mut s = SettingsScreen::new(&st, d, AppConfig::default()).open(Page::Global, &st);
        let f = frame(&mut s, &st, 80, 24);
        assert!(f.contains("Name"), "{f}");
        assert!(f.contains('…'), "{f}");
    }

    /// A failed read keeps what we had; only the device can take a row away,
    /// and only by answering differently at the next connect.
    #[test]
    fn wp_tm17_a_failed_re_read_keeps_the_last_values() {
        let mut have = extras();
        have.adopt(DeviceExtras {
            device_name: None,
            source_names: vec![None; 7],
            latency: None,
        });
        assert_eq!(have, extras());

        have.adopt(DeviceExtras::default());
        assert_eq!(have, extras(), "empty is a stall, not absence");

        let mut names = vec![None; 7];
        names[0] = Some("Mac".to_string());
        have.adopt(DeviceExtras {
            device_name: Some("New".into()),
            source_names: names,
            latency: Some(Latency {
                requested_id: 2,
                options: Vec::new(),
            }),
        });
        assert_eq!(have.device_name.as_deref(), Some("New"));
        assert_eq!(have.source_names[0].as_deref(), Some("Mac"));
        assert_eq!(have.source_names[1].as_deref(), Some("Living Room TV"));
        let latency = have.latency.expect("latency");
        assert_eq!(latency.requested_id, 2);
        assert_eq!(latency.options.len(), 7, "the table read failed");
    }
}
