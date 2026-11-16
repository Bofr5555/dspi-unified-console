//! Measurement documents in the core (WP P5DOCK, PLAN-P5 T3, T7, T11 and
//! section 1.6).
//!
//! The documents of kinds `measurement`, `mic` and `target` are
//! `dspi_measure::doc`'s; this module gives the clients one way to file,
//! list, read, rename, annotate and delete them, to make a `mic` document
//! from a calibration file and a `target` document from a target file, to
//! list the built-in targets, and to export a measured output as REW text.
//!
//! - **The local store** is `dspi_session::measure_store` (a folder per
//!   device serial for measurements, `mic/` and `target/`, an index, atomic
//!   writes, the corrupt-file rule, the snapshot store's ids rule), in the
//!   folder the client names with [`DspiCore::measure_set_dir`].
//! - **On the web** there are no files: the calls that take and give
//!   document text ([`mic_import`], [`target_import`], [`measure_doc_info`],
//!   [`DspiCore::measurement_export_rew`] with a document's text) are its way,
//!   as the snapshot document calls are; the store's calls list nothing and
//!   answer `Unsupported`. The hub's store is P5HUB's.
//! - **No device I/O:** nothing here reads or writes the device. A document
//!   configures no device: `config_import` and every other document path
//!   refuse one (`files.rs`, [`MEASURE_DOC_REFUSAL`]).
//!
//! It also holds the core's [`SectionModel`] ([`CoreSectionModel`]): the
//! measurement maths evaluate every chain with the curves' own cascade
//! (`curves.rs`), so a prediction and the graph never disagree about a
//! filter (OD-P5c).

use std::path::PathBuf;
use std::sync::Mutex;

use dspi_measure::cal::parse_calibration;
use dspi_measure::doc::{DocKind, Meta, MicCal, MicDoc, Target, TargetDoc};
use dspi_measure::export::{RewExport, rew_text};
use dspi_measure::target::{TargetUse, builtins, import_target};
use dspi_measure::types::{BandSpec, Coeffs, Coupler, LogGrid, MeasureError, SectionModel};
use dspi_proto::enums::FilterType;
use dspi_session::measure_store::{AnyDoc, MeasureEntry, valid_id};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use dspi_session::measure_store::MeasureStore;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::records::FilterParams;
use crate::{DspiCore, DspiError, Result};

/// What `config_import` and the other document paths say of a measurement
/// document (`files.rs`): it configures no device.
pub const MEASURE_DOC_REFUSAL: &str =
    "This is a measurement, microphone or target document: it configures no device. \
     Open it in Measurements instead.";

/// The longest file name a `mic` document keeps, characters.
const MAX_SOURCE_NAME_CHARS: usize = 256;

/// The folder the client named for measurement documents.
// The web has no files: the slot is never read there.
#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
#[derive(Debug, Default)]
pub(crate) struct DirSlot(Mutex<Option<PathBuf>>);

/// One measurement, microphone or target document in a list
/// ([`DspiCore::measure_docs`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDocInfo {
    /// The store's id, for the other calls ("" for a document not filed).
    pub id: String,
    /// The kind: `measurement`, `mic` or `target`.
    pub kind: String,
    /// The document's name; any Unicode.
    pub name: String,
    /// The user's note; any Unicode.
    pub note: String,
    /// The measured device's serial (measurements only, else "").
    pub serial: String,
    /// When it was measured (a measurement) or filed (the others), ms since
    /// the Unix epoch; 0 when not known.
    pub saved_ms: i64,
    /// The document's size, bytes.
    pub bytes: u64,
}

impl From<MeasureEntry> for MeasureDocInfo {
    fn from(e: MeasureEntry) -> Self {
        Self {
            id: e.id,
            kind: e.kind,
            name: e.name,
            note: e.note,
            serial: e.serial,
            saved_ms: e.saved_ms,
            bytes: e.bytes,
        }
    }
}

/// One parameter of a built-in target, with its default.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetParam {
    /// The parameter's name, as a `target` document's `params` holds it.
    pub name: String,
    /// Its default value.
    pub value: f64,
}

/// A built-in target ([`target_builtins`]): defined by parameters in our own
/// code, no third-party data (OD-P5p).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    /// The stable id a `target` document names it by (`flat`, `tilt`,
    /// `house`, `coupler-iec60318-4`, `coupler-flat-plate`, `coupler-hats`).
    pub id: String,
    /// The display name (it says the curve is ours).
    pub name: String,
    /// `speakers` or `headphones`.
    pub target_use: String,
    /// The coupler it is the default for (`iec60318_4`, `flatPlate`,
    /// `hats`), "" for a speaker target.
    pub coupler: String,
    /// Its parameters with their defaults (none for a fixed curve).
    pub params: Vec<TargetParam>,
}

// ─── The section model ──────────────────────────────────────────────────

/// The core's [`SectionModel`]: a band's sections are exactly the ones the
/// curves are computed from (`curves.rs`, [`crate::curve_sections`]), at the
/// curves' sample rate. Every type the curves know is modelled (the PEQ
/// shapes, the Linkwitz Transform and the crossover family); an unknown type
/// is not (no sections, `models` false). The measurement session, the tune
/// and the live view use it (P5SES, P5TUNE, P5LIVE).
// Used by the measurement session, the tune and the live view (P5SES, P5TUNE,
// P5LIVE), which land after this WP; until then only by its tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CoreSectionModel;

impl SectionModel for CoreSectionModel {
    fn sections(&self, band: &BandSpec) -> Vec<Coeffs> {
        if band.bypass || band.filter_type == 0 || !self.models(band) {
            return Vec::new();
        }
        crate::curves::curve_sections(vec![FilterParams {
            filter_type: band.filter_type,
            bypass: false,
            freq: band.freq,
            q: band.q,
            gain: band.gain,
            qp: band.qp,
        }])
        .into_iter()
        .map(|s| Coeffs {
            b0: s.b0,
            b1: s.b1,
            b2: s.b2,
            a1: s.a1,
            a2: s.a2,
        })
        .collect()
    }

    fn models(&self, band: &BandSpec) -> bool {
        FilterType::from_raw(band.filter_type).is_known()
            || dspi_proto::xover::meta(band.filter_type).is_some()
    }

    fn sample_rate(&self) -> f64 {
        dspi_proto::dsp::SAMPLE_RATE
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

fn measure_error(e: MeasureError) -> DspiError {
    invalid(e.to_string())
}

#[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
fn io_error(e: std::io::Error) -> DspiError {
    match e.kind() {
        std::io::ErrorKind::NotFound => invalid(format!("measurement document not found ({e})")),
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
            invalid(e.to_string())
        }
        _ => DspiError::Io {
            msg: format!("the measurement store: {e}"),
        },
    }
}

/// The kind a list call names: "" every kind.
fn kind_of(word: &str) -> Result<Option<DocKind>> {
    if word.trim().is_empty() {
        return Ok(None);
    }
    DocKind::from_word(word).map(Some).ok_or_else(|| {
        invalid(format!(
            "no document kind \"{}\" (measurement, mic or target)",
            word.chars().take(32).collect::<String>()
        ))
    })
}

/// A seam word (`speakers`, `iec60318_4`, …) as its type, by the type's own
/// serde spelling, so a call and a document spell it alike.
fn word_as<T: serde::de::DeserializeOwned>(word: &str, what: &str) -> Result<T> {
    serde_json::from_value(Value::String(word.trim().to_string())).map_err(|_| {
        invalid(format!(
            "no {what} \"{}\"",
            word.chars().take(32).collect::<String>()
        ))
    })
}

fn serde_word<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The `meta` a document made here carries.
fn meta(kind: DocKind, name: &str) -> Meta {
    Meta::new(
        kind,
        name,
        &crate::files::utc_now_text(),
        env!("CARGO_PKG_VERSION"),
    )
}

/// `s` cut to `max` characters, control characters as spaces.
fn clean(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

// ─── Free functions: no device, no files ────────────────────────────────

/// What a measurement, microphone or target document's text is: validated by
/// its kind's reader (bounded, every number in range), and described as a
/// list row. `bytes` is the text's size; `id` the document's own ("" when
/// not filed). No I/O: the web's way to list the documents it keeps.
///
/// Errors: [`DspiError::InvalidArgument`] for a text that is not a valid
/// document of the family (a preset, a snapshot, past its bound, damaged).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn measure_doc_info(text: String) -> Result<MeasureDocInfo> {
    let d = AnyDoc::parse(&text).map_err(measure_error)?;
    let id = d.id().to_string();
    Ok(d.entry(&id, u64::try_from(text.len()).unwrap_or(u64::MAX)).into())
}

/// A `mic` document from a microphone's calibration file (miniDSP UMIK-1 or
/// UMIK-2 text with its `Sens Factor` line, REW calibration text, or CSV),
/// on the standard grid: the file's format, serial, sensitivity and gain,
/// its angle (90° when the file name or a comment says so), the dB SPL scale
/// its sensitivity implies (labelled approximate until a calibrator step),
/// the file's name and SHA-256. Not filed: [`DspiCore::measure_doc_put`]
/// files it. `name` "" names it after the vendor and serial, else the file.
///
/// Errors: [`DspiError::InvalidArgument`] for a file that is not a
/// calibration this build reads (past 1 MiB, binary, no point in range), the
/// reason in words, never the file's contents.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn mic_import(name: String, file_name: String, file_text: String) -> Result<String> {
    let cal = parse_calibration(&file_text, &file_name).map_err(measure_error)?;
    let file_name = clean(&file_name, MAX_SOURCE_NAME_CHARS);
    let name = match clean(&name, dspi_measure::doc::MAX_NAME_CHARS) {
        n if !n.is_empty() => n,
        _ => {
            let parts: Vec<String> = [cal.vendor.clone(), cal.curve.serial.clone()]
                .into_iter()
                .flatten()
                .collect();
            if parts.is_empty() {
                file_name.clone()
            } else {
                parts.join(" ")
            }
        }
    };
    let mut mic = MicCal::from_calibration(&name, &cal, None, &LogGrid::STANDARD)
        .map_err(measure_error)?;
    mic.source_name = file_name;
    mic.source_hash = Sha256::digest(file_text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    MicDoc {
        schema_version: dspi_measure::doc::SCHEMA_VERSION,
        meta: meta(DocKind::Mic, &name),
        mic,
        extra: Map::new(),
    }
    .to_text()
    .map_err(measure_error)
}

/// A `target` document from a target file (REW or CSV text, a phase column
/// and a sensitivity line ignored), on the standard grid, normalised at the
/// default reference band (500 Hz to 2 kHz). `target_use` is `speakers` or
/// `headphones`; `coupler` (`iec60318_4`, `flatPlate`, `hats`, or "") is kept
/// for a headphone target only. Not filed: [`DspiCore::measure_doc_put`]
/// files it.
///
/// Errors: [`DspiError::InvalidArgument`] for an unknown use or coupler, or
/// a file that is not a target this build reads.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn target_import(
    name: String,
    file_text: String,
    target_use: String,
    coupler: String,
) -> Result<String> {
    let target_use: TargetUse = word_as(&target_use, "target use")?;
    let coupler: Option<Coupler> = if coupler.trim().is_empty() {
        None
    } else {
        Some(word_as(&coupler, "coupler")?)
    };
    let t = import_target(&file_text, target_use, coupler).map_err(measure_error)?;
    let name = clean(&name, dspi_measure::doc::MAX_NAME_CHARS);
    let target = Target::from_target(&name, &t, &LogGrid::STANDARD).map_err(measure_error)?;
    TargetDoc {
        schema_version: dspi_measure::doc::SCHEMA_VERSION,
        meta: meta(DocKind::Target, &name),
        target,
        extra: Map::new(),
    }
    .to_text()
    .map_err(measure_error)
}

/// Every built-in target with its default parameters, in a stable order:
/// flat, tilt and house for speakers, then each coupler's default.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn target_builtins() -> Vec<TargetInfo> {
    builtins()
        .into_iter()
        .map(|b| TargetInfo {
            id: b.id,
            name: b.name,
            target_use: serde_word(&b.target_use),
            coupler: b.coupler.as_ref().map(serde_word).unwrap_or_default(),
            params: b
                .builtin
                .params()
                .into_iter()
                .map(|(name, value)| TargetParam { name, value })
                .collect(),
        })
        .collect()
}

/// Output `output`'s response of a measurement document as REW text.
fn export_rew(text: &str, output: u8) -> Result<String> {
    let d = AnyDoc::parse(text).map_err(measure_error)?;
    let AnyDoc::Measurement(m) = d else {
        return Err(invalid(format!(
            "a {} document has no measured outputs",
            d.kind().word()
        )));
    };
    let o = m
        .measurement
        .outputs
        .iter()
        .find(|o| o.output == output)
        .ok_or_else(|| invalid(format!("the measurement has no output {output}")))?;
    let role = serde_word(&o.role);
    let source = if o.speaker.is_empty() {
        format!("Output {} ({role})", u16::from(output).saturating_add(1))
    } else {
        format!(
            "Output {} ({} {role})",
            u16::from(output).saturating_add(1),
            o.speaker
        )
    };
    let mut notes = vec![format!("Device: {}", m.measurement.serial)];
    if let Some(mic) = &m.measurement.mic {
        notes.push(format!(
            "Microphone: {}{}",
            mic.name,
            if mic.approximate && o.response.spl {
                " (dB SPL approximate)"
            } else {
                ""
            }
        ));
    }
    notes.push(format!(
        "Timing: {}",
        if m.measurement.capture.timing_reliable {
            "reliable"
        } else {
            "unreliable"
        }
    ));
    rew_text(
        &o.response.to_response(),
        &RewExport {
            title: m.meta.name.clone(),
            source,
            smoothing: None,
            notes,
            trusted_only: false,
        },
    )
    .map_err(measure_error)
}

// ─── The API on the core ────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Keep measurement, microphone and target documents in files under
    /// `dir`, the client's own data folder (created if missing): a folder per
    /// device serial for measurements, `mic/` and `target/`, each with an
    /// index. "" forgets the folder.
    ///
    /// Errors: [`DspiError::Io`] when the folder cannot be created;
    /// [`DspiError::Unsupported`] on the web (no files there).
    pub fn measure_set_dir(&self, dir: String) -> Result<()> {
        self.measure_dir_set(dir)
    }

    /// The filed documents of `kind` (`measurement`, `mic`, `target`; ""
    /// every kind): by kind, then oldest first. Measurements of device
    /// `serial` only ("" every device's). No folder named (and the web) is
    /// an empty list; a damaged index is rebuilt from the documents and a
    /// damaged document set aside (the corrupt-file rule).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown kind.
    pub fn measure_docs(&self, kind: String, serial: String) -> Result<Vec<MeasureDocInfo>> {
        let kind = kind_of(&kind)?;
        Ok(self.measure_list(kind, &serial))
    }

    /// Filed document `id`'s text.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown id, one two
    /// folders hold, or a damaged document (set aside); [`DspiError::Io`]
    /// with no folder named; [`DspiError::Unsupported`] on the web.
    pub fn measure_doc_get(&self, id: String) -> Result<String> {
        self.measure_read(&id)
    }

    /// File a measurement, microphone or target document (validated by its
    /// kind's reader) as a new document; a filed document is never
    /// overwritten. Its own id is kept when free, else one is made from its
    /// time. A measurement that names no device takes the open device's
    /// serial.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a text that is not a valid
    /// document of the family (a preset or a snapshot among them), or a
    /// measurement with no serial and no device open; [`DspiError::Io`] with
    /// no folder named, a failed write, or a full store (100 measurements a
    /// device, 32 microphones, 64 targets, 64 MiB);
    /// [`DspiError::Unsupported`] on the web.
    pub fn measure_doc_put(&self, text: String) -> Result<MeasureDocInfo> {
        self.measure_put(&text)
    }

    /// Rename filed document `id`; any Unicode, cut to 256 characters.
    /// Errors as [`DspiCore::measure_doc_get`], and [`DspiError::Io`].
    pub fn measure_doc_rename(&self, id: String, name: String) -> Result<MeasureDocInfo> {
        self.measure_update(&id, Some(name), None)
    }

    /// Set filed document `id`'s note; any Unicode, cut to 4096 characters.
    /// Errors as [`DspiCore::measure_doc_rename`].
    pub fn measure_doc_note(&self, id: String, note: String) -> Result<MeasureDocInfo> {
        self.measure_update(&id, None, Some(note))
    }

    /// Delete filed document `id`. Errors as [`DspiCore::measure_doc_get`].
    pub fn measure_doc_delete(&self, id: String) -> Result<()> {
        self.measure_delete(&id)
    }

    /// Output `output`'s measured response as REW text ("Import frequency
    /// response"): `doc` is a filed measurement's id or a measurement
    /// document's text (the web's way). Frequency, dB and the wrapped phase
    /// where measured; a header with the name, the output and the
    /// microphone. Derived curves only (S12).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown id, a document
    /// that is not a measurement, or an output it does not hold; as
    /// [`DspiCore::measure_doc_get`] for an id.
    pub fn measurement_export_rew(&self, doc: String, output: u8) -> Result<String> {
        let text = if valid_id(&doc) {
            self.measure_read(&doc)?
        } else {
            doc
        };
        export_rew(&text, output)
    }
}

// ─── The store ──────────────────────────────────────────────────────────

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
impl DspiCore {
    fn measure_store(&self) -> Result<MeasureStore> {
        let dir = self
            .measure_dir
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        dir.map(MeasureStore::new).ok_or_else(|| DspiError::Io {
            msg: "no folder has been named for measurement documents".into(),
        })
    }

    fn measure_dir_set(&self, dir: String) -> Result<()> {
        let dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).map_err(|e| DspiError::Io {
                msg: format!("the measurement documents' folder: {e}"),
            })?;
        }
        *self
            .measure_dir
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = dir;
        Ok(())
    }

    fn measure_list(&self, kind: Option<DocKind>, serial: &str) -> Vec<MeasureDocInfo> {
        let Ok(store) = self.measure_store() else {
            return Vec::new();
        };
        store
            .list(kind, Some(serial))
            .into_iter()
            .map(MeasureDocInfo::from)
            .collect()
    }

    fn measure_read(&self, id: &str) -> Result<String> {
        self.measure_store()?
            .read(id)
            .map(|(_, t)| t)
            .map_err(io_error)
    }

    fn measure_put(&self, text: &str) -> Result<MeasureDocInfo> {
        let store = self.measure_store()?;
        let mut d = AnyDoc::parse(text).map_err(measure_error)?;
        let text = match &mut d {
            AnyDoc::Measurement(m) if m.measurement.serial.is_empty() => {
                let open = self.lock().conn.as_ref().map(|c| c.serial.clone());
                m.measurement.serial = open.filter(|s| !s.is_empty()).ok_or_else(|| {
                    invalid("the measurement names no device, and none is open")
                })?;
                d.to_text().map_err(measure_error)?
            }
            _ => text.to_string(),
        };
        store
            .put(&text, crate::events::now_ms())
            .map(|(e, _)| e.into())
            .map_err(io_error)
    }

    fn measure_update(
        &self,
        id: &str,
        name: Option<String>,
        note: Option<String>,
    ) -> Result<MeasureDocInfo> {
        let store = self.measure_store()?;
        let e = match (name, note) {
            (Some(n), _) => store.rename(id, &n),
            (None, Some(n)) => store.set_note(id, &n),
            (None, None) => store.read(id).map(|(e, _)| e),
        };
        e.map(MeasureDocInfo::from).map_err(io_error)
    }

    fn measure_delete(&self, id: &str) -> Result<()> {
        self.measure_store()?.delete(id).map_err(io_error)
    }
}

/// The web has no files: the document calls are its store.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl DspiCore {
    fn measure_dir_set(&self, _dir: String) -> Result<()> {
        Err(DspiError::Unsupported)
    }
    fn measure_list(&self, _kind: Option<DocKind>, _serial: &str) -> Vec<MeasureDocInfo> {
        Vec::new()
    }
    fn measure_read(&self, _id: &str) -> Result<String> {
        Err(DspiError::Unsupported)
    }
    fn measure_put(&self, _text: &str) -> Result<MeasureDocInfo> {
        Err(DspiError::Unsupported)
    }
    fn measure_update(
        &self,
        _id: &str,
        _name: Option<String>,
        _note: Option<String>,
    ) -> Result<MeasureDocInfo> {
        Err(DspiError::Unsupported)
    }
    fn measure_delete(&self, _id: &str) -> Result<()> {
        Err(DspiError::Unsupported)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5dock;
