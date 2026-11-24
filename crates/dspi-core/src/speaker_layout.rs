//! The speaker layout (WP ROLE0, MPLAN section 2.2): which output drives
//! what, as the user confirmed it. It is the one source of output roles (full
//! range, sub, low, mid, high): the measurement takes its roles from it
//! (`MeasureSetup::roles_from_layout`). -- WP ROLES -- An agent writes it
//! only as roles stated in the user's words ([`DspiCore::speaker_roles_state`],
//! after the agents' policy asked the user), never the user's confirmation.
//!
//! - **Stored per device**, on the host: a file per serial under the folder
//!   of [`DspiCore::speaker_layout_set_dir`] (the limits folder's pattern, one
//!   subfolder per serial). With no folder named (and on the web, which has no
//!   files) a layout set is kept for this core's lifetime only; the page keeps
//!   its own copy and sets it again when it opens the device.
//! - **The proposal** ([`DspiCore::speaker_layout_propose`]) is the filters'
//!   inference (`dspi_session::output_label::driver_role`, the labels' one),
//!   never saved by itself.
//! - **Set by the user** ([`DspiCore::speaker_layout_set`]): the Getting
//!   Started step, Settings › Speakers and the Measure tables call it; an
//!   agent's tools never do. -- WP ROLES -- An agent's `dspi_set_speaker_roles`
//!   keeps the user's stated roles as a [`LayoutState::Stated`] layout.
//! - **It travels with a configuration**: `config_export` writes it in
//!   `meta.speakerLayout`, and an import of the crossover block keeps it for
//!   the open device.
//!
//! **Locks:** the slot is taken with or without `inner`, never `inner` after
//! it; the calls read what they need from `inner` first and release it.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::measure::MeasureRole;
use crate::{DspiCore, DspiError, Result};

/// The notice key of a measurement refused for want of a confirmed layout.
pub const NOTICE_LAYOUT_UNCONFIRMED: &str = "measure.layout_unconfirmed";
// -- WP ROLES --
/// The refusal key of roles stated over a confirmed layout with no words on
/// what changed.
pub const NOTICE_LAYOUT_CONFIRMED: &str = "layout.confirmed";
/// The block of an exported configuration's `meta` that holds the layout.
pub const META_BLOCK: &str = "speakerLayout";
/// The file in a device's subfolder.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
const FILE: &str = "speaker_layout.json";
/// The `kind` a layout file carries.
const KIND: &str = "speakerLayout";
/// The version a layout file is written with (-- WP ADET0 -- 2: rows' sources
/// and the detection; version 1 files still load).
const VERSION: u32 = 2;
/// A layout file larger than this is not one.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
const MAX_BYTES: usize = 64 * 1024;
/// Outputs in a layout at most (the registry's 16 channels).
const MAX_OUTPUTS: usize = 16;
/// A speaker's name, characters at most.
const MAX_SPEAKER_CHARS: usize = 64;

/// One output of the layout.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakerLayoutOutput {
    /// The output, 0-based.
    pub output: u8,
    /// What it drives.
    pub role: MeasureRole,
    /// The speaker it belongs to (`L`, `R`, `Sub`, or the user's word).
    pub speaker: String,
    /// Its alignment group: the outputs of one speaker share one.
    pub group: u8,
    // -- WP ADET0 --
    /// Where its role comes from: the user, the speaker detection, or a
    /// suggestion (a version 1 file's rows are the user's when the layout is
    /// confirmed, else suggestions).
    pub source: RoleSource,
}

/// Which output drives what, for one device.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakerLayout {
    /// The device's serial (set by the core; "" in an exported copy).
    pub serial: String,
    /// The outputs, each once, in output order.
    pub outputs: Vec<SpeakerLayoutOutput>,
    /// The user confirmed it (the Getting Started step, Settings ›
    /// Speakers). A proposal is never confirmed.
    pub confirmed_by_user: bool,
    /// When it was set, ms since the Unix epoch; 0 for a proposal.
    pub updated_ms: i64,
    /// Remarks in words (a proposal's: outputs left out and why).
    pub notes: Vec<String>,
    // -- WP ADET0 --
    /// The last speaker detection on this device, if one ran: its evidence
    /// per output. With every row `detected` or `user` it makes the layout
    /// [`LayoutState::AutoDetected`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub detected: Option<Detection>,
    // -- WP ROLES --
    /// The roles an AI agent set from the user's words, if it did: the
    /// words, when, and the top-fall checks made since. With a `stated` row
    /// and no suggestion it makes the layout [`LayoutState::Stated`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub stated: Option<Statement>,
}

// -- WP ADET0 -- the speaker detection's part of the layout (AUTODES 3.1)

/// Where a row's role comes from.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleSource {
    /// The user set or confirmed it.
    User,
    /// The speaker detection found it.
    Detected,
    /// A suggestion (the filters' inference, an import), nobody's word.
    #[default]
    Suggested,
    // -- WP ROLES --
    /// Set by an AI agent from the user's words (the layout's `stated`),
    /// not checked by the user.
    Stated,
}

/// How far the layout can be trusted ([`SpeakerLayout::state`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutState {
    /// Nobody's word: no roles reach the safety rules.
    #[default]
    Unconfirmed,
    /// Found by the speaker detection, not yet confirmed: its roles guard the
    /// outputs as a confirmed layout's do.
    AutoDetected,
    /// The user confirmed it.
    Confirmed,
    // -- WP ROLES --
    /// Set by an AI agent from the user's words, not yet confirmed: its
    /// roles guard the outputs as a detected layout's do, an output it does
    /// not name is unknown (a tweeter), and an agent's sub-floor stimulus on
    /// a stated non-tweeter waits for the top-fall check.
    Stated,
}

/// What the speaker detection made of an output.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectClass {
    /// A tweeter, or anything that plays flat to 20 kHz and is not probed
    /// lower (a full-range driver, a dome midrange): role high.
    Tweeter,
    /// Rolls off between 70 and 200 Hz: role mid.
    Midrange,
    /// Plays below 70 Hz and above 1 kHz: role low.
    Woofer,
    /// Plays below 70 Hz, 20 dB or more quieter at 1 to 2 kHz: role sub.
    Sub,
    /// A full-range driver: the user's correction only, never detected.
    FullRange,
    /// Nothing reached the microphone: left out of the layout.
    Nothing,
    /// The probe did not finish it (stopped, the microphone did not follow,
    /// the settings changed): role high.
    #[default]
    Unknown,
    /// The output was off or muted: not probed.
    Off,
}

/// How sure the detection is of a class.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectConfidence {
    /// The evidence is plain.
    High,
    /// Probably ("Tweeter?": flat to 20 kHz, could be a full-range driver).
    Medium,
    /// Uncertain, treated as the most protective class.
    #[default]
    Low,
}

/// Where the detection first heard an output.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectHeard {
    /// In the presence band, 4 to 16 kHz (stage A).
    Top,
    /// Between the floor and 4 kHz (stage A2).
    AboveFloor,
    /// Only by the shaped probe below the floor (stage C1).
    BelowFloor,
    /// Nowhere.
    #[default]
    Never,
}

/// One output's detection and its evidence.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DetectedOutput {
    /// The output, 0-based.
    pub output: u8,
    /// What it was found to be.
    pub class: DetectClass,
    /// How sure.
    pub confidence: DetectConfidence,
    /// The role it was given; `None` for `nothing` and `off`.
    pub role: Option<MeasureRole>,
    /// Its floor, Hz: nothing below it but the shaped probe.
    pub floor_hz: f32,
    /// Where it was first heard.
    pub heard: DetectHeard,
    /// The level it was heard at in the presence ramp, peak dBFS.
    pub level_dbfs: Option<f32>,
    /// The mean level 1.5 to 3 kHz less the mean 10 to 16 kHz, dB.
    pub top_fall_db: Option<f32>,
    /// How far the response has turned down in the half octave above the
    /// floor, dB below its loudest point.
    pub low_turn_db: Option<f32>,
    /// Where the response is 6 dB under its loudest point, Hz; `None` when
    /// not reached (or not looked for).
    pub f_low_hz: Option<f32>,
    /// The lowest frequency any stimulus played on it, Hz.
    pub lowest_hz: Option<f32>,
    /// The descent's segment it stopped at, Hz.
    pub stopped_at_hz: Option<f32>,
    /// A notice key (`detect.nothing`, `detect.off`, `detect.unknown`, ...).
    pub note: Option<String>,
    /// When it was probed, ms since the Unix epoch.
    pub probed_ms: i64,
}

/// A speaker detection's record in the layout.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Detection {
    /// When it started, ms since the Unix epoch.
    pub at_ms: i64,
    /// Who ran it: an agent's label, "" for the app (the user).
    pub by: String,
    /// Where the microphone was.
    pub placement: crate::measure::MeasurePlacement,
    /// The floor the caller asked for, Hz (each output's own may be higher).
    pub floor_hz: f32,
    /// The microphone's name ("" uncalibrated).
    pub microphone: String,
    /// Each output it was asked about, probed or not.
    pub outputs: Vec<DetectedOutput>,
    /// It was stopped before every output was probed.
    pub stopped: bool,
    /// The result in words.
    pub summary: String,
    /// The device's output count when it ran: another count invalidates it.
    pub device_outputs: u8,
}

// -- WP ROLES -- roles an agent set from the user's words (owner 2026-11-23)

/// The words a stated layout was set from, at most this many characters.
pub const MAX_WORDS_CHARS: usize = 1_000;

/// What the top-fall check (the detection's stage B) made of a stated
/// non-tweeter, before an agent plays anything below its floor.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleCheckVerdict {
    /// The top end falls as a woofer's or a midrange's does: the stated
    /// role stands.
    TopFalls,
    /// Nothing heard above the floor, but heard below it by the
    /// detection's shaped probe (at most −50 dBFS, two octaves): a sub or a
    /// low woofer, no tweeter; the stated role stands. (Heard nowhere, the
    /// check is [`RoleCheckVerdict::Unfinished`].)
    Silent,
    /// The top holds like a tweeter's: nothing below the floor plays on it
    /// for an agent, and the safety rules take it for a tweeter.
    TopHolds,
    /// The check did not finish (stopped, the microphone did not follow):
    /// still unchecked.
    #[default]
    Unfinished,
}

/// One output's top-fall check.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleCheck {
    /// The output, 0-based.
    pub output: u8,
    /// The stated role it checked (a check stands only for that role).
    pub role: MeasureRole,
    /// What it found.
    pub verdict: RoleCheckVerdict,
    /// The mean level 1.5 to 3 kHz less the mean 10 to 16 kHz, dB.
    pub top_fall_db: Option<f32>,
    /// The floor it never played below, Hz.
    pub floor_hz: f32,
    /// When, ms since the Unix epoch.
    pub at_ms: i64,
    /// Unfinished because the output's own filters (a low-pass) hide its top
    /// end: the check judges the driver, not them.
    pub filters_hide_top: bool,
}

/// The user's words an AI agent set the roles from.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Statement {
    /// When, ms since the Unix epoch.
    pub at_ms: i64,
    /// The agent's label ("AI · Claude Code").
    pub by: String,
    /// The user's words, as the agent quoted them.
    pub words: String,
    /// Over a confirmed layout: the user's words on what changed ("" else).
    pub what_changed: String,
    /// The device's output count then: another count invalidates it.
    pub device_outputs: u8,
    /// The top-fall checks made since, by output.
    pub checks: Vec<RoleCheck>,
}

impl SpeakerLayout {
    /// The output's row, if the layout has one.
    pub fn output(&self, output: u8) -> Option<&SpeakerLayoutOutput> {
        self.outputs.iter().find(|o| o.output == output)
    }

    // -- WP ADET0 --
    /// How far it can be trusted: confirmed by the user; else auto-detected
    /// when a detection is recorded and no row is a mere suggestion; else
    /// unconfirmed.
    pub fn state(&self) -> LayoutState {
        let nobodys = self.outputs.is_empty()
            || self
                .outputs
                .iter()
                .any(|o| o.source == RoleSource::Suggested);
        if self.confirmed_by_user {
            LayoutState::Confirmed
        } else if nobodys {
            LayoutState::Unconfirmed
        // -- WP ROLES -- an agent's word from the user's is the least checked
        } else if self.stated.is_some()
            && self.outputs.iter().any(|o| o.source == RoleSource::Stated)
        {
            LayoutState::Stated
        } else if self.detected.is_some() {
            LayoutState::AutoDetected
        } else {
            LayoutState::Unconfirmed
        }
    }

    // -- WP ROLES --
    /// The top-fall check of `output`'s stated row, if one was made for
    /// the role it has now.
    pub fn check_of(&self, output: u8) -> Option<&RoleCheck> {
        let row = self.output(output)?;
        self.stated
            .as_ref()?
            .checks
            .iter()
            .find(|c| c.output == output && c.role == row.role)
    }

    // -- WP ROLES --
    /// Whether an agent's stimulus on `output` stays above the floor: in a
    /// stated layout, an output it does not name (unknown, a tweeter), or a
    /// stated row whose top-fall check has not cleared it (not made, not
    /// finished, or the top held like a tweeter's). Never for the user's own
    /// rows, a detected or a confirmed layout.
    pub fn agent_floor_held(&self, output: u8) -> bool {
        if self.state() != LayoutState::Stated {
            return false;
        }
        let Some(row) = self.output(output) else {
            return true;
        };
        row.source == RoleSource::Stated
            && row.role != MeasureRole::High
            && !self.check_of(output).is_some_and(|c| {
                matches!(
                    c.verdict,
                    RoleCheckVerdict::TopFalls | RoleCheckVerdict::Silent
                )
            })
    }

    // -- WP ROLES --
    /// The stated non-tweeter rows an agent's stimulus below the floor waits
    /// for a check on, among `outputs`: no check made yet for their role (a
    /// check that found a tweeter's top is not made again).
    pub fn unchecked(&self, outputs: &[u8]) -> Vec<u8> {
        if self.state() != LayoutState::Stated {
            return Vec::new();
        }
        outputs
            .iter()
            .copied()
            .filter(|o| {
                self.output(*o)
                    .is_some_and(|r| r.source == RoleSource::Stated && r.role != MeasureRole::High)
                    && self
                        .check_of(*o)
                        .is_none_or(|c| c.verdict == RoleCheckVerdict::Unfinished)
            })
            .collect()
    }

    // -- WP ADET0 --
    /// The detection's record of `output`, if one.
    pub fn detected_output(&self, output: u8) -> Option<&DetectedOutput> {
        self.detected
            .as_ref()
            .and_then(|d| d.outputs.iter().find(|o| o.output == output))
    }

    // -- WP ADET0 --
    /// A detection made when the device had another output count no longer
    /// stands: its rows read as suggestions (kept, not trusted).
    pub(crate) fn checked_against(mut self, outputs: u8) -> Self {
        if self
            .detected
            .as_ref()
            .is_some_and(|d| d.device_outputs != outputs)
        {
            for o in &mut self.outputs {
                if o.source == RoleSource::Detected {
                    o.source = RoleSource::Suggested;
                }
            }
        }
        // -- WP ROLES -- so is a statement made for another output count
        if self
            .stated
            .as_ref()
            .is_some_and(|s| s.device_outputs != outputs)
        {
            for o in &mut self.outputs {
                if o.source == RoleSource::Stated {
                    o.source = RoleSource::Suggested;
                }
            }
        }
        self
    }

    /// Checked and put in order for `serial` on a device of `outputs`
    /// outputs: no output twice or past the device, at most 16, short
    /// speaker names.
    fn normalised(mut self, serial: &str, outputs: u8) -> Result<Self> {
        if self.outputs.len() > MAX_OUTPUTS {
            return Err(invalid(format!(
                "a speaker layout has at most {MAX_OUTPUTS} outputs"
            )));
        }
        self.outputs.sort_by_key(|o| o.output);
        for (i, o) in self.outputs.iter().enumerate() {
            let n = u16::from(o.output).saturating_add(1);
            if o.output >= outputs {
                return Err(invalid(format!("the DSPi has no output {n}")));
            }
            if i.checked_sub(1)
                .and_then(|p| self.outputs.get(p))
                .is_some_and(|p| p.output == o.output)
            {
                return Err(invalid(format!("output {n} twice in the speaker layout")));
            }
            if o.speaker.chars().count() > MAX_SPEAKER_CHARS {
                return Err(invalid(format!(
                    "output {n}'s speaker name is longer than {MAX_SPEAKER_CHARS} characters"
                )));
            }
        }
        self.serial = serial.to_owned();
        Ok(self)
    }

    /// The layout as a file's document.
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
    pub(crate) fn to_document(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Some(o) = v.as_object_mut() {
            o.insert("kind".into(), json!(KIND));
            o.insert("version".into(), json!(VERSION)); // -- WP ADET0 --
        }
        v
    }

    /// A layout from a file's document or an export's `meta.speakerLayout`.
    pub(crate) fn from_value(v: &Value) -> std::result::Result<Self, String> {
        let o = v.as_object().ok_or("not a JSON object")?;
        if let Some(k) = o.get("kind")
            && k.as_str() != Some(KIND)
        {
            return Err(format!("not a speaker layout (kind {k})"));
        }
        let mut l: SpeakerLayout = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        // -- WP ADET0 -- a version 1 row has no source: the user's in a
        // confirmed layout, else a suggestion
        let rows = o.get("outputs").and_then(Value::as_array);
        for (row, value) in l.outputs.iter_mut().zip(rows.into_iter().flatten()) {
            if value.get("source").is_none() {
                row.source = if l.confirmed_by_user {
                    RoleSource::User
                } else {
                    RoleSource::Suggested
                };
            }
        }
        Ok(l)
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// Where layouts are kept.
#[derive(Debug, Default)]
struct Store {
    /// The folder the client named (none on the web).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    dir: Option<std::path::PathBuf>,
    /// Layouts set with no folder named, by serial, for this core's lifetime.
    memory: Vec<SpeakerLayout>,
}

/// The core's layouts (a field of [`DspiCore`]).
#[derive(Debug, Default)]
pub(crate) struct LayoutSlot(Mutex<Store>);

impl LayoutSlot {
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `serial`'s layout: its file, else (no folder) the one kept in memory.
    /// A file that cannot be read is set aside and reads as none.
    pub(crate) fn load(&self, serial: &str) -> Option<SpeakerLayout> {
        let store = self.store();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(dir) = &store.dir {
            // -- WP MDQT1 -- one that could not be written to it is in memory still
            if let Some(l) = files::load(dir, serial) {
                return Some(l);
            }
        }
        store.memory.iter().find(|l| l.serial == serial).cloned()
    }

    /// Keep `layout` for its serial.
    pub(crate) fn save(&self, layout: &SpeakerLayout) -> Result<()> {
        // -- WP ADETS -- the function test's oracle judges each output by it
        #[cfg(feature = "sim-scenario")]
        crate::measure_sim::scenario::layout_saved(layout);
        let mut store = self.store();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(dir) = store.dir.clone() {
            files::save(&dir, layout).map_err(|e| DspiError::Io {
                msg: format!("the speaker layout could not be written: {e}"),
            })?;
            store.memory.retain(|l| l.serial != layout.serial); // -- WP MDQT1 --
            return Ok(());
        }
        store.memory.retain(|l| l.serial != layout.serial);
        store.memory.push(layout.clone());
        Ok(())
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod files {
    //! The file side, gated off on the web.

    use std::fs;
    use std::io;
    use std::path::Path;

    use dspi_session::preset_file::save_file;
    use dspi_session::snapshot_store::dir_name;

    use super::{FILE, MAX_BYTES, SpeakerLayout};

    pub(super) fn load(root: &Path, serial: &str) -> Option<SpeakerLayout> {
        let path = root.join(dir_name(serial)).join(FILE);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("the speaker layout could not be read: {e}");
                return None;
            }
        };
        let parsed = if bytes.len() > MAX_BYTES {
            Err("the file is too large".to_owned())
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|v| SpeakerLayout::from_value(&v))
        };
        match parsed {
            Ok(mut l) => {
                l.serial = serial.to_owned();
                Some(l)
            }
            Err(why) => {
                // The corrupt-file rule: set aside, never a panic, reads as none.
                log::warn!("the speaker layout file is not one ({why}): set aside");
                let _ = fs::rename(&path, path.with_extension("json.bad"));
                None
            }
        }
    }

    pub(super) fn save(root: &Path, layout: &SpeakerLayout) -> io::Result<()> {
        let dir = root.join(dir_name(&layout.serial));
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&dir)?;
        let text = serde_json::to_string_pretty(&layout.to_document()).map_err(io::Error::other)?;
        save_file(&dir.join(FILE), &text)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Keep speaker layouts in files under `dir`, the app's own data folder,
    /// one subfolder per device serial (created when first written; "" forgets
    /// the folder). Layouts set before a folder was named are written to it
    /// where it holds none for that device.
    ///
    /// Errors: [`DspiError::Unsupported`] on the web (no files: the page keeps
    /// the layout and sets it again with [`DspiCore::speaker_layout_set`]);
    /// -- WP MDQT1 -- [`DspiError::Io`] when a layout set before could not be
    /// written there: the folder is named all the same, and that layout stays
    /// in force in memory until a later call or a save writes it.
    pub fn speaker_layout_set_dir(&self, dir: String) -> Result<()> {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut store = self.speaker_layouts.store();
            store.dir = (!dir.trim().is_empty()).then(|| dir.into());
            if let Some(d) = store.dir.clone() {
                let mut failed = Ok(());
                for l in std::mem::take(&mut store.memory) {
                    if files::load(&d, &l.serial).is_none()
                        && let Err(e) = files::save(&d, &l)
                    {
                        log::warn!("a speaker layout could not be written to the new folder: {e}");
                        // -- WP MDQT1 -- kept, and said (review RA)
                        store.memory.push(l);
                        failed = Err(DspiError::Io {
                            msg: format!("the speaker layout could not be written: {e}"),
                        });
                    }
                }
                return failed;
            }
            Ok(())
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = dir;
            Err(DspiError::Unsupported)
        }
    }

    /// The open device's speaker layout as the user set it; `None` when none
    /// was set (show [`DspiCore::speaker_layout_propose`] to confirm). Reads
    /// the host's file, not the device.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device.
    pub fn speaker_layout(&self) -> Result<Option<SpeakerLayout>> {
        let (serial, outputs) = {
            let mut inner = self.lock();
            (
                inner.conn()?.serial.clone(),
                inner.state.num_output_channels,
            )
        };
        // -- WP ADET0 -- a detection of another output count no longer stands
        Ok(self
            .speaker_layouts
            .load(&serial)
            .map(|l| l.checked_against(outputs)))
    }

    /// What the device's filters and routing suggest (the inference the
    /// output labels use): each enabled output that plays an input, its role
    /// from its filters (or "sub" in its name), its speaker from its
    /// programme source, one group per speaker. Never confirmed, never saved.
    /// Reads the mirror only.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device.
    pub fn speaker_layout_propose(&self) -> Result<SpeakerLayout> {
        let serial = self.lock().conn()?.serial.clone();
        let m = crate::measure::layout_proposal_of(&self.get_state());
        Ok(SpeakerLayout {
            serial,
            outputs: m
                .outputs
                .into_iter()
                .map(|o| SpeakerLayoutOutput {
                    output: o.output,
                    role: o.role,
                    speaker: o.speaker,
                    group: o.group,
                    source: RoleSource::Suggested, // -- WP ADET0 --
                })
                .collect(),
            confirmed_by_user: false,
            updated_ms: 0,
            notes: m.notes,
            detected: None, // -- WP ADET0 --
            stated: None,   // -- WP ROLES --
        })
    }

    /// Keep `layout` as the open device's speaker layout, stamped now, with
    /// its `confirmed_by_user` as given. **For the user's own screens only**
    /// (the Getting Started step, Settings › Speakers, the Measure tables):
    /// an agent never sets roles (MPLAN 2.2). Returns what was kept.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InvalidArgument`] for an output twice, one the device
    /// lacks, more than 16, or a speaker name over 64 characters (nothing
    /// kept); [`DspiError::Io`] when the file cannot be written;
    /// [`DspiError::ConfirmationRequired`] `user_only` inside another actor's
    /// [`crate::actor::ActorScope`] (WP INT1).
    pub fn speaker_layout_set(&self, layout: SpeakerLayout) -> Result<SpeakerLayout> {
        // -- WP INT1 -- only the user sets it (also on an agent's import)
        crate::actor::user_only(self, "The speaker layout")?;
        let (serial, outputs) = {
            let mut inner = self.lock();
            let serial = inner.conn()?.serial.clone();
            (serial, inner.state.num_output_channels)
        };
        let mut layout = layout.normalised(&serial, outputs)?;
        layout.updated_ms = crate::events::now_ms();
        let before = self.speaker_layouts.load(&serial);
        // -- WP REVDFIX -- B3: the statement on record is the core's (its
        // checks are written by the checks alone), never the caller's
        layout.stated = before.as_ref().and_then(|b| b.stated.clone());
        // -- WP ADET0 -- a confirmed row is the user's word
        if layout.confirmed_by_user {
            for o in &mut layout.outputs {
                // -- WP ROLES -- so is a row an agent set from the user's words
                if matches!(o.source, RoleSource::Suggested | RoleSource::Stated) {
                    o.source = RoleSource::User;
                }
            }
        }
        // -- WP ADETS -- (AUTODES 3.1) a detected row the user gave another role is
        // the user's word from now on; the detection stands for the other rows
        for o in &mut layout.outputs {
            let was = before.as_ref().and_then(|b| b.output(o.output));
            // -- WP ROLES -- or a stated one
            if matches!(o.source, RoleSource::Detected | RoleSource::Stated)
                && was.is_some_and(|w| w.role != o.role)
            {
                o.source = RoleSource::User;
            }
        }
        self.speaker_layouts.save(&layout)?;
        // -- WP AIVIS -- an agent learns that its roles have moved
        let act = if layout.confirmed_by_user {
            "speakers_confirmed"
        } else {
            "speakers"
        };
        // -- WP ADET0 -- with what moved, in numbers and role words
        crate::actor::user::did_with(self, act, &role_changes(before.as_ref(), &layout));
        Ok(layout)
    }
}

impl DspiCore {
    // -- WP ADET0 -- `confirmed_speaker_layout` and `confirmed_layout_now`
    // (ROLE0, RCFIX) became the trusted layout's calls below: a detected
    // layout's roles guard the outputs as a confirmed one's do.

    // -- WP ADET0 --
    /// The open device's layout if its roles can be trusted (confirmed by the
    /// user, or found by the speaker detection); `None` otherwise.
    pub(crate) fn trusted_layout_now(&self) -> Option<SpeakerLayout> {
        self.speaker_layout()
            .ok()
            .flatten()
            .filter(|l| l.state() != LayoutState::Unconfirmed)
    }

    // -- WP ADET0 --
    /// The open device's layout if its roles can be trusted, else the
    /// `measure.layout_unconfirmed` refusal.
    pub(crate) fn trusted_speaker_layout(&self) -> Result<SpeakerLayout> {
        self.speaker_layout()?
            .filter(|l| l.state() != LayoutState::Unconfirmed)
            .ok_or_else(|| {
                invalid(format!(
                    "{NOTICE_LAYOUT_UNCONFIRMED}: the speaker layout is neither confirmed nor \
                     detected: ask the user to confirm it in Settings › Speakers, or detect \
                     the speakers"
                ))
            })
    }

    // -- WP ADET0 --
    /// Keep a layout the speaker detection wrote (not the user's act: no
    /// `user_only`, no `did`).
    pub(crate) fn speaker_layout_keep_detected(&self, layout: &SpeakerLayout) -> Result<()> {
        let (serial, outputs) = {
            let mut inner = self.lock();
            (
                inner.conn()?.serial.clone(),
                inner.state.num_output_channels,
            )
        };
        let mut layout = layout.clone().normalised(&serial, outputs)?;
        layout.updated_ms = crate::events::now_ms();
        self.speaker_layouts.save(&layout)
    }

    // -- WP ROLES --
    /// Set the open device's speaker roles **as an AI agent, from the
    /// user's words** (owner 2026-11-23; the agents' policy asked the user
    /// first, or the user's switch let it): `rows` as stated (source
    /// `stated`), `words` the user's words the agent quotes, and over a
    /// confirmed layout `what_changed`, the user's words on what changed.
    /// The layout is then [`LayoutState::Stated`]: trusted as a detected one
    /// is, an output it does not name unknown (a tweeter), and an agent's
    /// stimulus below a stated non-tweeter's floor waits for its top-fall
    /// check ([`SpeakerLayout::agent_floor_held`]). A check made for a row
    /// whose role did not change is kept. Returns what was kept.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InvalidArgument`] for no rows, no words, words over
    /// [`MAX_WORDS_CHARS`], a confirmed layout without `what_changed`, or
    /// the rows' faults of [`DspiCore::speaker_layout_set`] (nothing kept);
    /// [`DspiError::Io`] when the file cannot be written.
    pub fn speaker_roles_state(
        &self,
        rows: Vec<SpeakerLayoutOutput>,
        words: String,
        what_changed: String,
    ) -> Result<SpeakerLayout> {
        let actor = crate::actor::JobActor::capture(self).actor();
        let (serial, outputs) = {
            let mut inner = self.lock();
            let serial = inner.conn()?.serial.clone();
            (serial, inner.state.num_output_channels)
        };
        let words = words.trim().to_owned();
        let what_changed = what_changed.trim().to_owned();
        if rows.is_empty() {
            return Err(invalid("no speaker roles were stated"));
        }
        if words.is_empty() {
            return Err(invalid(
                "the user's words the roles come from are missing: quote them",
            ));
        }
        if words.chars().count() > MAX_WORDS_CHARS || what_changed.chars().count() > MAX_WORDS_CHARS
        {
            return Err(invalid(format!(
                "the user's words are longer than {MAX_WORDS_CHARS} characters"
            )));
        }
        let before = self.speaker_layout()?;
        if before
            .as_ref()
            .is_some_and(|l| l.state() == LayoutState::Confirmed)
            && what_changed.is_empty()
        {
            return Err(invalid(format!(
                "{NOTICE_LAYOUT_CONFIRMED}: the user confirmed the speaker layout: it is set again \
                 only from the user's words on what changed"
            )));
        }
        let stated_rows = rows
            .into_iter()
            .map(|r| SpeakerLayoutOutput {
                source: RoleSource::Stated,
                ..r
            })
            .collect();
        let mut layout = SpeakerLayout {
            outputs: stated_rows,
            ..SpeakerLayout::default()
        }
        .normalised(&serial, outputs)?;
        // A check stands while the role it checked does (-- WP REVDFIX --
        // B3: and the stated row it was made on: same role, same speaker;
        // never one carried in by an import or another layout)
        let checks = before
            .as_ref()
            .filter(|b| b.state() == LayoutState::Stated)
            .and_then(|b| b.stated.as_ref().map(|st| (b, st)))
            .map(|(b, st)| {
                st.checks
                    .iter()
                    .filter(|c| {
                        let was = b.output(c.output);
                        layout.output(c.output).is_some_and(|r| {
                            r.role == c.role
                                && was.is_some_and(|w| {
                                    w.source == RoleSource::Stated
                                        && w.role == r.role
                                        && w.speaker == r.speaker
                                })
                        })
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        layout.detected = before.as_ref().and_then(|b| b.detected.clone());
        layout.stated = Some(Statement {
            at_ms: crate::events::now_ms(),
            by: crate::actor::actor_label(actor).unwrap_or_default(),
            words,
            what_changed,
            device_outputs: outputs,
            checks,
        });
        layout.updated_ms = crate::events::now_ms();
        self.speaker_layouts.save(&layout)?;
        Ok(layout)
    }

    // -- WP ROLES --
    /// Record the top-fall check `check` in the stated layout (when its row
    /// still has the role it checked).
    pub(crate) fn speaker_roles_checked(&self, check: RoleCheck) {
        let Ok(Some(mut layout)) = self.speaker_layout() else {
            return;
        };
        if layout.state() != LayoutState::Stated
            || !layout
                .output(check.output)
                .is_some_and(|r| r.source == RoleSource::Stated && r.role == check.role)
        {
            return;
        }
        let Some(st) = layout.stated.as_mut() else {
            return;
        };
        st.checks.retain(|c| c.output != check.output);
        st.checks.push(check);
        st.checks.sort_by_key(|c| c.output);
        if let Err(e) = self.speaker_layouts.save(&layout) {
            log::warn!("the speaker roles' check could not be kept: {e}");
        }
    }

    /// `text`, an export, with the open device's layout in
    /// `meta.speakerLayout` (no serial); unchanged with none.
    pub(crate) fn with_speaker_layout_meta(&self, text: String) -> String {
        let Ok(Some(mut layout)) = self.speaker_layout() else {
            return text;
        };
        let Ok(mut doc) = serde_json::from_str::<Value>(&text) else {
            return text;
        };
        layout.serial.clear();
        let Some(meta) = doc
            .as_object_mut()
            .and_then(|o| o.entry("meta").or_insert_with(|| json!({})).as_object_mut())
        else {
            return text;
        };
        meta.insert(
            META_BLOCK.into(),
            serde_json::to_value(&layout).unwrap_or(Value::Null),
        );
        serde_json::to_string_pretty(&doc).unwrap_or(text)
    }

    /// Keep an imported document's `meta.speakerLayout` for the open device:
    /// outputs it lacks are dropped, the rest kept as the document has them
    /// but always **unconfirmed**, whatever the file says: the export blanks
    /// the serial, so an import cannot know it is the same speaker, and roles
    /// come from the user's own setup step (review RA F1). Returns whether one
    /// was kept.
    pub(crate) fn import_speaker_layout(&self, doc: &Value) -> bool {
        let Some(v) = doc.get("meta").and_then(|m| m.get(META_BLOCK)) else {
            return false;
        };
        let mut layout = match SpeakerLayout::from_value(v) {
            Ok(l) => l,
            Err(why) => {
                log::warn!("the document's speaker layout is not one ({why}): left out");
                return false;
            }
        };
        let outputs = self.lock().state.num_output_channels;
        layout.outputs.retain(|o| o.output < outputs);
        layout.outputs.truncate(MAX_OUTPUTS);
        layout.outputs.sort_by_key(|o| o.output);
        layout.outputs.dedup_by_key(|o| o.output);
        for o in &mut layout.outputs {
            if o.speaker.chars().count() > MAX_SPEAKER_CHARS {
                o.speaker = o.speaker.chars().take(MAX_SPEAKER_CHARS).collect();
            }
        }
        layout.notes.clear();
        layout.confirmed_by_user = false;
        // -- WP ADET0 -- another speaker's detection is nobody's word here
        layout.detected = None;
        // -- WP REVDFIX -- B3: nor another speaker's statement and its checks
        layout.stated = None;
        for o in &mut layout.outputs {
            o.source = RoleSource::Suggested;
        }
        match self.speaker_layout_set(layout) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("the document's speaker layout could not be kept: {e}");
                false
            }
        }
    }
}

// -- WP ADET0 --
/// What a newly set layout changed against `before`, in numbers and role
/// words only ("Out 3 midrange (was tweeter?)"); "" for nothing or no
/// layout before.
pub(crate) fn role_changes(before: Option<&SpeakerLayout>, after: &SpeakerLayout) -> String {
    let Some(before) = before else {
        return String::new();
    };
    let mut words = Vec::new();
    for row in &after.outputs {
        let n = u16::from(row.output).saturating_add(1);
        match before.output(row.output) {
            Some(old) if old.role == row.role => {}
            Some(old) => {
                let unsure = before
                    .detected_output(row.output)
                    .is_some_and(|d| d.confidence != DetectConfidence::High);
                words.push(format!(
                    "Out {n} {} (was {}{})",
                    role_word(row.role),
                    role_word(old.role),
                    if unsure { "?" } else { "" }
                ));
            }
            None => words.push(format!("Out {n} {} (added)", role_word(row.role))),
        }
    }
    for old in &before.outputs {
        if after.output(old.output).is_none() {
            words.push(format!(
                "Out {} left out",
                u16::from(old.output).saturating_add(1)
            ));
        }
    }
    words.join(", ")
}

// -- WP ADET0 --
/// A role in the user's words.
pub fn role_word(role: MeasureRole) -> &'static str {
    match role {
        MeasureRole::Full => "full range",
        MeasureRole::Sub => "subwoofer",
        MeasureRole::Low => "woofer",
        MeasureRole::Mid => "midrange",
        MeasureRole::High => "tweeter",
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_role0;
// -- WP ADETS --
#[cfg(test)]
mod tests_wp_adets;
// -- WP REVDFIX --
#[cfg(test)]
mod tests_wp_revdfix;
