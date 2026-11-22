//! The speaker layout (WP ROLE0, MPLAN section 2.2): which output drives
//! what, as the user confirmed it. It is the one source of output roles (full
//! range, sub, low, mid, high): the measurement takes its roles from it
//! (`MeasureSetup::roles_from_layout`), and no agent ever writes it.
//!
//! - **Stored per device**, on the host: a file per serial under the folder
//!   of [`DspiCore::speaker_layout_set_dir`] (the limits folder's pattern, one
//!   subfolder per serial). With no folder named (and on the web, which has no
//!   files) a layout set is kept for this core's lifetime only; the page keeps
//!   its own copy and sets it again when it opens the device.
//! - **The proposal** ([`DspiCore::speaker_layout_propose`]) is the filters'
//!   inference (`dspi_session::output_label::driver_role`, the labels' one),
//!   never saved by itself.
//! - **Set by the user only** ([`DspiCore::speaker_layout_set`]): the
//!   Getting Started step, Settings › Speakers and the Measure tables call it;
//!   an agent's tools never do.
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
/// The block of an exported configuration's `meta` that holds the layout.
pub const META_BLOCK: &str = "speakerLayout";
/// The file in a device's subfolder.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
const FILE: &str = "speaker_layout.json";
/// The `kind` a layout file carries.
const KIND: &str = "speakerLayout";
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
}

/// Which output drives what, for one device.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}

impl SpeakerLayout {
    /// The output's row, if the layout has one.
    pub fn output(&self, output: u8) -> Option<&SpeakerLayoutOutput> {
        self.outputs.iter().find(|o| o.output == output)
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
    fn to_document(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Some(o) = v.as_object_mut() {
            o.insert("kind".into(), json!(KIND));
            o.insert("version".into(), json!(1));
        }
        v
    }

    /// A layout from a file's document or an export's `meta.speakerLayout`.
    fn from_value(v: &Value) -> std::result::Result<Self, String> {
        let o = v.as_object().ok_or("not a JSON object")?;
        if let Some(k) = o.get("kind")
            && k.as_str() != Some(KIND)
        {
            return Err(format!("not a speaker layout (kind {k})"));
        }
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
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
            return files::load(dir, serial);
        }
        store.memory.iter().find(|l| l.serial == serial).cloned()
    }

    /// Keep `layout` for its serial.
    pub(crate) fn save(&self, layout: &SpeakerLayout) -> Result<()> {
        let mut store = self.store();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(dir) = &store.dir {
            return files::save(dir, layout).map_err(|e| DspiError::Io {
                msg: format!("the speaker layout could not be written: {e}"),
            });
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
    /// the layout and sets it again with [`DspiCore::speaker_layout_set`]).
    pub fn speaker_layout_set_dir(&self, dir: String) -> Result<()> {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut store = self.speaker_layouts.store();
            store.dir = (!dir.trim().is_empty()).then(|| dir.into());
            if let Some(d) = store.dir.clone() {
                for l in std::mem::take(&mut store.memory) {
                    if files::load(&d, &l.serial).is_none()
                        && let Err(e) = files::save(&d, &l)
                    {
                        log::warn!("a speaker layout could not be written to the new folder: {e}");
                    }
                }
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
        let serial = self.lock().conn()?.serial.clone();
        Ok(self.speaker_layouts.load(&serial))
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
                })
                .collect(),
            confirmed_by_user: false,
            updated_ms: 0,
            notes: m.notes,
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
    /// kept); [`DspiError::Io`] when the file cannot be written.
    pub fn speaker_layout_set(&self, layout: SpeakerLayout) -> Result<SpeakerLayout> {
        let (serial, outputs) = {
            let mut inner = self.lock();
            let serial = inner.conn()?.serial.clone();
            (serial, inner.state.num_output_channels)
        };
        let mut layout = layout.normalised(&serial, outputs)?;
        layout.updated_ms = crate::events::now_ms();
        self.speaker_layouts.save(&layout)?;
        Ok(layout)
    }
}

impl DspiCore {
    /// The open device's layout if the user confirmed it, else the
    /// `measure.layout_unconfirmed` refusal.
    pub(crate) fn confirmed_speaker_layout(&self) -> Result<SpeakerLayout> {
        self.speaker_layout()?
            .filter(|l| l.confirmed_by_user)
            .ok_or_else(|| {
                invalid(format!(
                    "{NOTICE_LAYOUT_UNCONFIRMED}: the speaker layout is not confirmed: ask the \
                     user to confirm it in Settings › Speakers"
                ))
            })
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
    /// (confirmed or not). Returns whether one was kept.
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
        match self.speaker_layout_set(layout) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("the document's speaker layout could not be kept: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_role0;
