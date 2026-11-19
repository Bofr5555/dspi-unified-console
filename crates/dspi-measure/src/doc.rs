//! The documents of kinds `measurement`, `mic` and `target` (WP P5MDOC,
//! PLAN-P5 section 1.6, OD-P5g, review RC7).
//!
//! All three are `.dspipreset` schema-1 documents: `{"schemaVersion": 1,
//! "meta": {"kind": …, "name": …, "savedUtc": …, "appVersion": …}, <block>}`
//! where the block is `measurement`, `mic` or `target` (the kind's word).
//! They configure no device; `dspi-core`'s `config_import` refuses them.
//! The contract for integrators is `docs/integrator/measurement.md`.
//!
//! Every document is **untrusted input**: a reader takes any text without a
//! panic, bounded before it is parsed (`measurement` 1 MiB, `mic` and
//! `target` 256 KiB), and refuses a document of another kind. A document that
//! parses is then [validated](MeasurementDoc::validate): grids of at most
//! [`MAX_GRID_POINTS`](crate::types::MAX_GRID_POINTS), at most
//! [`MAX_OUTPUTS`] outputs and [`MAX_POSITIONS`] positions, names of at most
//! [`MAX_NAME_CHARS`] characters and notes of at most [`MAX_NOTE_CHARS`],
//! every number finite and within its range. Writers validate first, so
//! nothing written is refused when read back.
//!
//! **Unknown keys are kept**: every object this module defines carries the
//! keys it does not know in an `extra` map and writes them back unchanged, so
//! a later build's additions survive an edit by this one. The objects of the
//! shared seam types (a band of a chain, the programme feed, a grid, the
//! placement) have a fixed shape: unknown keys inside them are dropped.
//!
//! Curves are compact: see [`Curve`].

mod curve;

pub use curve::{Curve, MAG_STEP_DB, MAX_MAG_DB, MAX_PHASE_DEG, PHASE_STEP_DEG};

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{BandSpec, ChannelChain, Coupler, MeasureError, Placement, WayRole};

/// The `.dspipreset` schema these documents belong to; any other is refused.
pub const SCHEMA_VERSION: u64 = 1;
/// The largest `measurement` document, bytes.
pub const MAX_MEASUREMENT_BYTES: usize = 1 << 20;
/// The largest `mic` document, bytes.
pub const MAX_MIC_BYTES: usize = 256 << 10;
/// The largest `target` document, bytes.
pub const MAX_TARGET_BYTES: usize = 256 << 10;
/// The most outputs a measurement holds (and the highest output index + 1).
pub const MAX_OUTPUTS: usize = 16;
/// The most positions (or reseats) a measurement averages.
pub const MAX_POSITIONS: u8 = 9;
/// The longest name, characters.
pub const MAX_NAME_CHARS: usize = 256;
/// The longest note, characters.
pub const MAX_NOTE_CHARS: usize = 4096;
/// The longest other text (a serial, a platform, a hash, a vendor, a file
/// name, a builtin's id), characters.
pub const MAX_TEXT_CHARS: usize = 256;
/// The longest store id, bytes (the snapshot store's rule).
pub const MAX_ID_LEN: usize = 64;
/// The most bands in one bank of a stored chain.
pub const MAX_BANDS: usize = 32;
/// The most tune records a measurement keeps.
pub const MAX_TUNES: usize = 64;
/// The most parameters of a built-in target.
pub const MAX_BUILTIN_PARAMS: usize = 32;

/// Which document of the measurement family a text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocKind {
    /// A measurement run and its responses.
    Measurement,
    /// A microphone's calibration.
    Mic,
    /// A target curve.
    Target,
}

impl DocKind {
    /// Every kind, in a fixed order.
    pub const ALL: [DocKind; 3] = [DocKind::Measurement, DocKind::Mic, DocKind::Target];

    /// The word in `meta.kind`, which is also the key of the document's block.
    pub fn word(self) -> &'static str {
        match self {
            DocKind::Measurement => "measurement",
            DocKind::Mic => "mic",
            DocKind::Target => "target",
        }
    }

    /// The kind a `meta.kind` word names (ASCII case and surrounding space
    /// ignored); `None` for any other word, including the preset family's.
    pub fn from_word(word: &str) -> Option<DocKind> {
        let w = word.trim();
        DocKind::ALL
            .into_iter()
            .find(|k| k.word().eq_ignore_ascii_case(w))
    }

    /// The largest document of this kind, bytes.
    pub fn max_bytes(self) -> usize {
        match self {
            DocKind::Measurement => MAX_MEASUREMENT_BYTES,
            DocKind::Mic => MAX_MIC_BYTES,
            DocKind::Target => MAX_TARGET_BYTES,
        }
    }
}

/// The kind of a document's text: `None` for a text past
/// [`MAX_MEASUREMENT_BYTES`], not JSON, or of another kind (a preset, a
/// snapshot). A cheap sort for a store or an import; the kind's own reader
/// still checks everything.
pub fn doc_kind(text: &str) -> Option<DocKind> {
    if text.len() > MAX_MEASUREMENT_BYTES {
        return None;
    }
    let v: Value = serde_json::from_str(strip_bom(text)).ok()?;
    v.pointer("/meta/kind")
        .and_then(Value::as_str)
        .and_then(DocKind::from_word)
}

/// The `meta` object every document of the family carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// The kind's word ([`DocKind::word`]).
    pub kind: String,
    /// The document's name, as lists show it.
    #[serde(default)]
    pub name: String,
    /// When it was saved, ISO-8601 UTC.
    #[serde(default)]
    pub saved_utc: String,
    /// The writing application's version.
    #[serde(default)]
    pub app_version: String,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Meta {
    /// A `meta` of kind `kind`.
    pub fn new(kind: DocKind, name: &str, saved_utc: &str, app_version: &str) -> Meta {
        Meta {
            kind: kind.word().to_string(),
            name: name.to_string(),
            saved_utc: saved_utc.to_string(),
            app_version: app_version.to_string(),
            extra: Map::new(),
        }
    }

    fn validate(&self, kind: DocKind) -> Result<(), MeasureError> {
        if DocKind::from_word(&self.kind) != Some(kind) {
            return Err(MeasureError::Invalid(format!(
                "meta.kind is not \"{}\"",
                kind.word()
            )));
        }
        text("meta.name", &self.name, MAX_NAME_CHARS)?;
        text("meta.savedUtc", &self.saved_utc, MAX_TEXT_CHARS)?;
        text("meta.appVersion", &self.app_version, MAX_TEXT_CHARS)
    }
}

// ── measurement ───────────────────────────────────────────────────────────

/// A `measurement` document: one run (or the average of a run per position)
/// of one device's outputs, its responses and the chains they were measured
/// through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementDoc {
    /// Always [`SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The family's `meta` (kind `measurement`).
    pub meta: Meta,
    /// The measurement.
    pub measurement: Measurement,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `measurement` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    /// The store's id, when filed (the snapshot store's id rule).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The measured device's serial.
    pub serial: String,
    /// The device's platform (`RP2040`, `RP2350`).
    #[serde(default)]
    pub platform: String,
    /// When the run ended, ms since the Unix epoch.
    pub saved_ms: i64,
    /// Where the microphone was.
    pub placement: Placement,
    /// The microphone, when one was named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic: Option<MicRef>,
    /// What the generator played.
    pub stimulus: Stimulus,
    /// How the microphone's capture went.
    pub capture: CaptureInfo,
    /// One entry per measured output, 1 to [`MAX_OUTPUTS`], each output once.
    pub outputs: Vec<OutputMeasurement>,
    /// Positions (or reseats) averaged, 1 to [`MAX_POSITIONS`].
    #[serde(default = "one")]
    pub positions: u8,
    /// The user's note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Tunes applied from this measurement, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tunes: Vec<TuneRecord>,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> u8 {
    1
}

/// The microphone a measurement was taken with (its `mic` document's id and
/// what the measurement needs of it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicRef {
    /// The `mic` document's store id, when filed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The microphone's name.
    #[serde(default)]
    pub name: String,
    /// The microphone's serial, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// The `mic` document's `sensitivityDb` when calibrated, dB, in its
    /// format's convention (dBV/Pa for a `dayton` file, WP P5DAYTON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity_db: Option<f64>,
    /// The dB SPL scale is approximate (no calibrator step was done).
    #[serde(default)]
    pub approximate: bool,
    /// The calibration's hash (its `mic` document's `sourceHash`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cal_hash: String,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The kind of generator run (PLAN-P5 section 1.1, review RC11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunKind {
    /// Log sweeps, one output per cycle round-robin, repeated passes.
    Walked,
    /// The outputs of one transition at once (a correlated copy).
    Summed,
    /// One output, one sweep (the live view).
    LiveSweep,
    /// Finite pink noise on one output (the microphone RTA).
    Pink,
}

/// What the generator played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stimulus {
    /// The run's kind.
    pub kind: RunKind,
    /// Sweep start, Hz.
    pub f1_hz: f64,
    /// Sweep end, Hz.
    pub f2_hz: f64,
    /// One sweep (or the noise), ms: 1000 to 15000 for a sweep, at most
    /// 60000 for pink noise.
    pub duration_ms: u32,
    /// Passes, 1 to 8.
    pub repeat: u8,
    /// Gap between cycles, ms (at most 10000).
    pub gap_ms: u32,
    /// The generator's peak level, dBFS (-120 to 0).
    pub level_db: f64,
    /// The device's sample rate, Hz (44100, 48000 or 96000).
    pub device_rate_hz: u32,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// How the microphone's capture went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureInfo {
    /// The capture's sample rate, Hz (8000 to 384000).
    pub rate_hz: u32,
    /// The measured clock ratio, microphone to device (within ±1 %).
    pub clock_ratio: f64,
    /// The run passed the timing quality gate: its delays and phases may be
    /// used for alignment (PLAN-P5 section 1.1 step 7).
    pub timing_reliable: bool,
    /// The largest cycle residual from the timing fit, µs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_residual_us: Option<f64>,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One measured output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputMeasurement {
    /// The output, 0-based (below [`MAX_OUTPUTS`]).
    pub output: u8,
    /// Its way in the speaker.
    pub role: WayRole,
    /// The speaker it belongs to, as the layout names it (`L`, `R`, `Sub`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    /// The chain it was measured through, and its programme feed.
    pub chain: StoredChain,
    /// The arrival relative to its own cycle start, ms (timing-reliable runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_ms: Option<f64>,
    /// The response (with phase where known, the trusted mask, and the
    /// spread across positions or reseats).
    pub response: Curve,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A [`ChannelChain`] as a document holds it, with the chain object's unknown
/// keys kept (the bands' and the feed's shapes are fixed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredChain {
    /// The chain.
    #[serde(flatten)]
    pub chain: ChannelChain,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl From<ChannelChain> for StoredChain {
    fn from(chain: ChannelChain) -> Self {
        StoredChain {
            chain,
            extra: Map::new(),
        }
    }
}

/// A tune applied from a measurement: the snapshots it was filed between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneRecord {
    /// When it was applied, ms since the Unix epoch.
    pub at: i64,
    /// The "before" snapshot's id.
    pub snapshot_before: String,
    /// The "after" snapshot's id, when kept.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snapshot_after: String,
    /// Kept (confirmed) rather than reverted.
    pub kept: bool,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MeasurementDoc {
    /// Reads a `measurement` document: bounded, parsed, validated.
    ///
    /// Errors: [`MeasureError::Invalid`] past [`MAX_MEASUREMENT_BYTES`], for
    /// another schema or kind, or a value out of its bounds;
    /// [`MeasureError::Parse`] for text that is not such a document.
    pub fn from_text(text: &str) -> Result<Self, MeasureError> {
        let d: Self = read(text, DocKind::Measurement)?;
        d.validate()?;
        Ok(d)
    }

    /// The document's text (pretty JSON, as a store files it), validated
    /// first and at most [`MAX_MEASUREMENT_BYTES`].
    pub fn to_text(&self) -> Result<String, MeasureError> {
        self.validate()?;
        write(&self.canonical(), DocKind::Measurement)
    }

    /// Checks every bound (see the module's notes).
    pub fn validate(&self) -> Result<(), MeasureError> {
        schema(self.schema_version)?;
        self.meta.validate(DocKind::Measurement)?;
        let m = &self.measurement;
        id("measurement.id", &m.id)?;
        text("measurement.serial", &m.serial, MAX_TEXT_CHARS)?;
        text("measurement.platform", &m.platform, MAX_TEXT_CHARS)?;
        text("measurement.note", &m.note, MAX_NOTE_CHARS)?;
        placement(&m.placement)?;
        if !(1..=MAX_POSITIONS).contains(&m.positions) {
            return Err(MeasureError::Invalid(format!(
                "measurement.positions: {} (1 to {MAX_POSITIONS})",
                m.positions
            )));
        }
        if let Some(mic) = &m.mic {
            id("measurement.mic.id", &mic.id)?;
            text("measurement.mic.name", &mic.name, MAX_NAME_CHARS)?;
            text("measurement.mic.serial", &mic.serial, MAX_TEXT_CHARS)?;
            text("measurement.mic.calHash", &mic.cal_hash, MAX_TEXT_CHARS)?;
            opt_num(
                "measurement.mic.sensitivityDb",
                mic.sensitivity_db,
                -200.0,
                200.0,
            )?;
        }
        stimulus(&m.stimulus)?;
        capture(&m.capture)?;
        if m.outputs.is_empty() || m.outputs.len() > MAX_OUTPUTS {
            return Err(MeasureError::Invalid(format!(
                "measurement.outputs: {} entries (1 to {MAX_OUTPUTS})",
                m.outputs.len()
            )));
        }
        let mut seen: u32 = 0;
        for o in &m.outputs {
            let bit = 1u32
                .checked_shl(u32::from(o.output))
                .filter(|_| usize::from(o.output) < MAX_OUTPUTS)
                .ok_or_else(|| {
                    MeasureError::Invalid(format!(
                        "measurement.outputs: output {} (0 to {})",
                        o.output,
                        MAX_OUTPUTS.saturating_sub(1)
                    ))
                })?;
            if seen & bit != 0 {
                return Err(MeasureError::Invalid(format!(
                    "measurement.outputs: output {} twice",
                    o.output
                )));
            }
            seen |= bit;
            text("measurement.outputs.speaker", &o.speaker, MAX_TEXT_CHARS)?;
            chain(&o.chain.chain)?;
            opt_num(
                "measurement.outputs.arrivalMs",
                o.arrival_ms,
                -10_000.0,
                10_000.0,
            )?;
            o.response.validate("measurement.outputs.response")?;
        }
        if m.tunes.len() > MAX_TUNES {
            return Err(MeasureError::Invalid(format!(
                "measurement.tunes: {} (at most {MAX_TUNES})",
                m.tunes.len()
            )));
        }
        for t in &m.tunes {
            id("measurement.tunes.snapshotBefore", &t.snapshot_before)?;
            id("measurement.tunes.snapshotAfter", &t.snapshot_after)?;
        }
        Ok(())
    }
}

// ── mic ───────────────────────────────────────────────────────────────────

/// A `mic` document: one microphone's calibration on a grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicDoc {
    /// Always [`SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The family's `meta` (kind `mic`).
    pub meta: Meta,
    /// The calibration.
    pub mic: MicCal,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The calibration file's format it was imported from: `umik`, `rew`,
/// `csv` or `dayton` (WP P5DAYTON). P5CAL's own type (review P5-A: one enum, not a copy).
pub type MicFormat = crate::cal::CalFormat;

/// The `mic` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicCal {
    /// The store's id, when filed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The microphone's name.
    pub name: String,
    /// Its serial, when the file names one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// Its vendor, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    /// The format the calibration was imported from.
    pub format: MicFormat,
    /// The calibration's angle of incidence, degrees: 0 or 90.
    #[serde(default)]
    pub angle: u16,
    /// The file's sensitivity, dB (the vendor's convention): dBFS at 94 dB
    /// SPL for `umik` and `rew`, dBV/Pa at 1 kHz for `dayton` (WP P5DAYTON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity_db: Option<f64>,
    /// The file's gain, dB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
    /// The offset from full scale to dB SPL, dB, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spl_offset_db: Option<f64>,
    /// The dB SPL scale is approximate (no calibrator step was done).
    #[serde(default)]
    pub approximate: bool,
    /// The correction on a grid (magnitude, and phase when the file had it).
    pub response: Curve,
    /// The imported file's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_name: String,
    /// The imported file's hash.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_hash: String,
    /// The user's note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl MicDoc {
    /// Reads a `mic` document: bounded ([`MAX_MIC_BYTES`]), parsed, validated.
    /// Errors as [`MeasurementDoc::from_text`].
    pub fn from_text(text: &str) -> Result<Self, MeasureError> {
        let d: Self = read(text, DocKind::Mic)?;
        d.validate()?;
        Ok(d)
    }

    /// The document's text, validated first and at most [`MAX_MIC_BYTES`].
    pub fn to_text(&self) -> Result<String, MeasureError> {
        self.validate()?;
        write(&self.canonical(), DocKind::Mic)
    }

    /// Checks every bound.
    pub fn validate(&self) -> Result<(), MeasureError> {
        schema(self.schema_version)?;
        self.meta.validate(DocKind::Mic)?;
        let m = &self.mic;
        id("mic.id", &m.id)?;
        text("mic.name", &m.name, MAX_NAME_CHARS)?;
        text("mic.serial", &m.serial, MAX_TEXT_CHARS)?;
        text("mic.vendor", &m.vendor, MAX_TEXT_CHARS)?;
        text("mic.sourceName", &m.source_name, MAX_TEXT_CHARS)?;
        text("mic.sourceHash", &m.source_hash, MAX_TEXT_CHARS)?;
        text("mic.note", &m.note, MAX_NOTE_CHARS)?;
        if m.angle != 0 && m.angle != 90 {
            return Err(MeasureError::Invalid(format!(
                "mic.angle: {} (0 or 90)",
                m.angle
            )));
        }
        opt_num("mic.sensitivityDb", m.sensitivity_db, -200.0, 200.0)?;
        opt_num("mic.gainDb", m.gain_db, -200.0, 200.0)?;
        opt_num("mic.splOffsetDb", m.spl_offset_db, -300.0, 300.0)?;
        m.response.validate("mic.response")
    }
}

// ── target ────────────────────────────────────────────────────────────────

/// A `target` document: a target curve, built-in by parameters or imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDoc {
    /// Always [`SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The family's `meta` (kind `target`).
    pub meta: Meta,
    /// The target.
    pub target: Target,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub use crate::target::TargetUse;

/// The `target` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// The store's id, when filed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The target's name.
    pub name: String,
    /// What it is for.
    #[serde(rename = "use")]
    pub use_for: TargetUse,
    /// The coupler it is for (headphone targets only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupler: Option<Coupler>,
    /// The curve.
    pub definition: TargetDefinition,
    /// A coupler compensation subtracted from measurements before the fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensation: Option<Curve>,
    /// The band the target is normalised at.
    pub reference_band: ReferenceBand,
    /// The user's note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// How a target's curve is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum TargetDefinition {
    /// One of the built-in parametric targets, by its id and parameters.
    Builtin(BuiltinTarget),
    /// Imported points, on a grid.
    Imported(ImportedTarget),
}

/// A built-in target: its id and parameters (`dspi-measure`'s `target`
/// module defines both; a document only names them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinTarget {
    /// The built-in's id (`flat`, `tilt`, `house`, a coupler's default).
    pub builtin: String,
    /// Its parameters by name, at most [`MAX_BUILTIN_PARAMS`], each finite.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An imported target, resampled to a grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTarget {
    /// The curve (magnitude only is enough).
    pub curve: Curve,
    /// The imported file's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_name: String,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The band a target is normalised at, Hz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceBand {
    /// The band's low edge, Hz.
    pub low_hz: f64,
    /// The band's high edge, Hz (above `low_hz`, at most 100 kHz).
    pub high_hz: f64,
    /// Keys this build does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TargetDoc {
    /// Reads a `target` document: bounded ([`MAX_TARGET_BYTES`]), parsed,
    /// validated. Errors as [`MeasurementDoc::from_text`].
    pub fn from_text(text: &str) -> Result<Self, MeasureError> {
        let d: Self = read(text, DocKind::Target)?;
        d.validate()?;
        Ok(d)
    }

    /// The document's text, validated first and at most [`MAX_TARGET_BYTES`].
    pub fn to_text(&self) -> Result<String, MeasureError> {
        self.validate()?;
        write(&self.canonical(), DocKind::Target)
    }

    /// Checks every bound.
    pub fn validate(&self) -> Result<(), MeasureError> {
        schema(self.schema_version)?;
        self.meta.validate(DocKind::Target)?;
        let t = &self.target;
        id("target.id", &t.id)?;
        text("target.name", &t.name, MAX_NAME_CHARS)?;
        text("target.note", &t.note, MAX_NOTE_CHARS)?;
        if t.coupler.is_some() && t.use_for != TargetUse::Headphones {
            return Err(MeasureError::Invalid(
                "target.coupler: only a headphone target has one".into(),
            ));
        }
        match &t.definition {
            TargetDefinition::Builtin(b) => {
                if b.builtin.is_empty() {
                    return Err(MeasureError::Invalid(
                        "target.definition.builtin: empty".into(),
                    ));
                }
                text("target.definition.builtin", &b.builtin, MAX_TEXT_CHARS)?;
                if b.params.len() > MAX_BUILTIN_PARAMS {
                    return Err(MeasureError::Invalid(format!(
                        "target.definition.params: {} (at most {MAX_BUILTIN_PARAMS})",
                        b.params.len()
                    )));
                }
                for (k, v) in &b.params {
                    text("target.definition.params", k, MAX_TEXT_CHARS)?;
                    num("target.definition.params", *v, -1e6, 1e6)?;
                }
            }
            TargetDefinition::Imported(i) => {
                text(
                    "target.definition.sourceName",
                    &i.source_name,
                    MAX_TEXT_CHARS,
                )?;
                i.curve.validate("target.definition.curve")?;
            }
        }
        if let Some(c) = &t.compensation {
            c.validate("target.compensation")?;
        }
        let b = &t.reference_band;
        num("target.referenceBand.lowHz", b.low_hz, 0.0, 100_000.0)?;
        num("target.referenceBand.highHz", b.high_hz, 0.0, 100_000.0)?;
        if !(b.low_hz > 0.0 && b.high_hz > b.low_hz) {
            return Err(MeasureError::Invalid(
                "target.referenceBand: needs 0 < lowHz < highHz".into(),
            ));
        }
        Ok(())
    }
}

// ── review P5-A: P5CAL's types to and from the documents ──────────────────
//
// The document model and P5CAL were written in parallel. These are the only
// conversions between them, so a `mic` or `target` document always means what
// `cal` and `target` compute.

impl MicCal {
    /// The `mic` block of a parsed calibration on `grid` (normally
    /// [`LogGrid::STANDARD`](crate::types::LogGrid::STANDARD)): the file's
    /// format, angle, serial, vendor, sensitivity and gain, its response, and
    /// the dB SPL scale `cal::spl_scale` gives (a calibrator step's offset
    /// when there is one, else the sensitivity's, labelled approximate).
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid.
    pub fn from_calibration(
        name: &str,
        cal: &crate::cal::MicCalibration,
        calibrator: Option<crate::cal::CalibratorStep>,
        grid: &crate::types::LogGrid,
    ) -> Result<MicCal, MeasureError> {
        let scale = crate::cal::spl_scale(cal.curve.spl_sensitivity_db(), calibrator);
        Ok(MicCal {
            id: String::new(),
            name: name.to_string(),
            serial: cal.curve.serial.clone().unwrap_or_default(),
            vendor: cal.vendor.clone().unwrap_or_default(),
            format: cal.curve.format,
            angle: cal.curve.angle.degrees(),
            sensitivity_db: cal.curve.stated_sensitivity_db(),
            gain_db: cal.curve.gain_db,
            spl_offset_db: scale.offset_db,
            approximate: scale.approximate,
            response: Curve::from_response(&cal.to_response(grid)?),
            source_name: String::new(),
            source_hash: String::new(),
            note: String::new(),
            extra: Map::new(),
        })
    }

    /// The calibration's angle of incidence; `None` for a number that is not
    /// 0 or 90 (which [`MicDoc::validate`] refuses).
    pub fn mic_angle(&self) -> Option<crate::cal::MicAngle> {
        crate::cal::MicAngle::from_degrees(self.angle)
    }

    /// The correction to subtract from a measurement
    /// (`cal::apply_correction`), on the document's grid.
    pub fn correction(&self) -> crate::types::Response {
        self.response.to_response()
    }

    /// The dB SPL scale the document states: `dB SPL = dBFS +
    /// splOffsetDb` in the crate's level convention
    /// ([`FULL_SCALE_SINE_DB`](crate::types::FULL_SCALE_SINE_DB)); relative
    /// dB without an offset.
    pub fn spl_scale(&self) -> crate::cal::SplScale {
        use crate::cal::{SplBasis, SplScale};
        match self.spl_offset_db.filter(|o| o.is_finite()) {
            None => SplScale {
                offset_db: None,
                approximate: true,
                basis: SplBasis::Relative,
            },
            Some(offset) => SplScale {
                offset_db: Some(offset),
                approximate: self.approximate,
                // A document keeps the offset, not which calibrator gave it.
                basis: if self.approximate {
                    SplBasis::Sensitivity
                } else {
                    SplBasis::Calibrator {
                        reference_db: crate::cal::CALIBRATOR_94_DB,
                    }
                },
            },
        }
    }
}

impl From<crate::target::ReferenceBand> for ReferenceBand {
    fn from(b: crate::target::ReferenceBand) -> Self {
        ReferenceBand {
            low_hz: b.low_hz,
            high_hz: b.high_hz,
            extra: Map::new(),
        }
    }
}

impl Target {
    /// The `target` block of one of P5CAL's targets. A built-in is stored by
    /// its id and named parameters (`target::Builtin::id`, `params`); an
    /// imported one as its curve on `grid`.
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid or definition.
    pub fn from_target(
        name: &str,
        target: &crate::target::Target,
        grid: &crate::types::LogGrid,
    ) -> Result<Target, MeasureError> {
        let definition = match &target.definition {
            crate::target::TargetDefinition::Builtin { builtin } => {
                builtin.curve()?;
                TargetDefinition::Builtin(BuiltinTarget {
                    builtin: builtin.id(),
                    params: builtin.params(),
                    extra: Map::new(),
                })
            }
            crate::target::TargetDefinition::Imported { points } => {
                let mut r = crate::cal::curve_to_grid(points, grid)?;
                r.phase_deg = None;
                TargetDefinition::Imported(ImportedTarget {
                    curve: Curve::from_response(&r),
                    source_name: String::new(),
                    extra: Map::new(),
                })
            }
        };
        Ok(Target {
            id: String::new(),
            name: name.to_string(),
            use_for: target.target_use,
            coupler: target.coupler,
            definition,
            compensation: None,
            reference_band: target.reference.into(),
            note: String::new(),
            extra: Map::new(),
        })
    }

    /// The target as P5CAL evaluates it (`target::Target::evaluate`).
    ///
    /// Errors: [`MeasureError::Invalid`] for a built-in id or parameter name
    /// `target` does not define, a parameter out of range, or an imported
    /// curve with no trusted point.
    pub fn to_target(&self) -> Result<crate::target::Target, MeasureError> {
        let definition = match &self.definition {
            TargetDefinition::Builtin(b) => crate::target::TargetDefinition::Builtin {
                builtin: crate::target::Builtin::from_id(&b.builtin, &b.params)?,
            },
            TargetDefinition::Imported(i) => {
                let points = i.curve.points()?;
                if points.is_empty() {
                    return Err(MeasureError::Invalid(
                        "target.definition.curve: no trusted point".into(),
                    ));
                }
                crate::target::TargetDefinition::Imported { points }
            }
        };
        Ok(crate::target::Target {
            target_use: self.use_for,
            coupler: self.coupler,
            definition,
            reference: crate::target::ReferenceBand {
                low_hz: self.reference_band.low_hz,
                high_hz: self.reference_band.high_hz,
            },
        })
    }

    /// The coupler compensation as P5CAL takes it, when the target has one
    /// and names its coupler.
    pub fn to_compensation(&self) -> Result<Option<crate::target::Compensation>, MeasureError> {
        match (&self.compensation, self.coupler) {
            (Some(c), Some(coupler)) => Ok(Some(crate::target::Compensation {
                coupler,
                points: c.points()?,
            })),
            _ => Ok(None),
        }
    }
}

// ── review P5-A (F3): a written document reads back to the same text ──────
//
// `serde_json` reads a number of 16 or 17 digits to within one unit in the
// last place (its exact parser is a build-wide feature this crate does not
// switch on), so a document read and written again could differ in the last
// digit of a number: a store would see a changed document where nothing
// changed. Every free number is therefore written with 15 significant
// digits, which read back exactly; curves are integers already.

/// `x` at 15 significant digits (itself when it is not finite).
fn sig15(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

fn sig15_opt(x: &mut Option<f64>) {
    if let Some(v) = x {
        *v = sig15(*v);
    }
}

fn sig15_curve(c: &mut Curve) {
    c.grid.min_hz = sig15(c.grid.min_hz);
    c.grid.max_hz = sig15(c.grid.max_hz);
}

impl MeasurementDoc {
    /// The document as it is written: every free number at 15 significant
    /// digits, so that reading the text and writing it again gives the same
    /// text.
    pub fn canonical(&self) -> MeasurementDoc {
        let mut d = self.clone();
        let m = &mut d.measurement;
        m.stimulus.f1_hz = sig15(m.stimulus.f1_hz);
        m.stimulus.f2_hz = sig15(m.stimulus.f2_hz);
        m.stimulus.level_db = sig15(m.stimulus.level_db);
        m.capture.clock_ratio = sig15(m.capture.clock_ratio);
        sig15_opt(&mut m.capture.max_residual_us);
        if let Some(mic) = m.mic.as_mut() {
            sig15_opt(&mut mic.sensitivity_db);
        }
        for o in &mut m.outputs {
            sig15_opt(&mut o.arrival_ms);
            sig15_curve(&mut o.response);
        }
        d
    }
}

impl MicDoc {
    /// The document as it is written (see [`MeasurementDoc::canonical`]).
    pub fn canonical(&self) -> MicDoc {
        let mut d = self.clone();
        sig15_opt(&mut d.mic.sensitivity_db);
        sig15_opt(&mut d.mic.gain_db);
        sig15_opt(&mut d.mic.spl_offset_db);
        sig15_curve(&mut d.mic.response);
        d
    }
}

impl TargetDoc {
    /// The document as it is written (see [`MeasurementDoc::canonical`]).
    pub fn canonical(&self) -> TargetDoc {
        let mut d = self.clone();
        let t = &mut d.target;
        match &mut t.definition {
            TargetDefinition::Builtin(b) => b.params.values_mut().for_each(|v| *v = sig15(*v)),
            TargetDefinition::Imported(i) => sig15_curve(&mut i.curve),
        }
        if let Some(c) = t.compensation.as_mut() {
            sig15_curve(c);
        }
        t.reference_band.low_hz = sig15(t.reference_band.low_hz);
        t.reference_band.high_hz = sig15(t.reference_band.high_hz);
        d
    }
}

// ── any kind ──────────────────────────────────────────────────────────────

/// A document of any of the three kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum MeasureDocument {
    /// A `measurement`.
    Measurement(Box<MeasurementDoc>),
    /// A `mic`.
    Mic(Box<MicDoc>),
    /// A `target`.
    Target(Box<TargetDoc>),
}

impl MeasureDocument {
    /// Reads a document of whichever kind its `meta.kind` names, with that
    /// kind's bounds. Errors as [`MeasurementDoc::from_text`]; a document of
    /// none of the three kinds is [`MeasureError::Invalid`].
    pub fn from_text(text: &str) -> Result<Self, MeasureError> {
        if text.len() > MAX_MEASUREMENT_BYTES {
            return Err(too_large(text.len(), MAX_MEASUREMENT_BYTES));
        }
        let v = parse_value(text)?;
        let word = v.pointer("/meta/kind").and_then(Value::as_str);
        match word.and_then(DocKind::from_word) {
            Some(DocKind::Measurement) => {
                MeasurementDoc::from_text(text).map(|d| Self::Measurement(Box::new(d)))
            }
            Some(DocKind::Mic) => MicDoc::from_text(text).map(|d| Self::Mic(Box::new(d))),
            Some(DocKind::Target) => TargetDoc::from_text(text).map(|d| Self::Target(Box::new(d))),
            None => Err(MeasureError::Invalid(format!(
                "a {} document, not a measurement, mic or target",
                kind_label(word)
            ))),
        }
    }

    /// The document's kind.
    pub fn kind(&self) -> DocKind {
        match self {
            Self::Measurement(_) => DocKind::Measurement,
            Self::Mic(_) => DocKind::Mic,
            Self::Target(_) => DocKind::Target,
        }
    }

    /// The document's text (its kind's `to_text`).
    pub fn to_text(&self) -> Result<String, MeasureError> {
        match self {
            Self::Measurement(d) => d.to_text(),
            Self::Mic(d) => d.to_text(),
            Self::Target(d) => d.to_text(),
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn too_large(len: usize, max: usize) -> MeasureError {
    MeasureError::Invalid(format!("a document of {len} bytes (at most {max})"))
}

fn parse_value(text: &str) -> Result<Value, MeasureError> {
    let v: Value = serde_json::from_str(strip_bom(text))
        .map_err(|e| MeasureError::Parse(format!("not JSON: {e}")))?;
    if v.is_object() {
        Ok(v)
    } else {
        Err(MeasureError::Parse("not a JSON object".into()))
    }
}

/// The kind word as an error names it: the preset family's default when
/// absent, and never more than 32 characters of untrusted text.
fn kind_label(word: Option<&str>) -> String {
    match word {
        None => "preset".into(),
        Some(w) => format!("\"{}\"", w.chars().take(32).collect::<String>()),
    }
}

fn read<T: DeserializeOwned>(text: &str, kind: DocKind) -> Result<T, MeasureError> {
    if text.len() > kind.max_bytes() {
        return Err(too_large(text.len(), kind.max_bytes()));
    }
    let v = parse_value(text)?;
    schema(v.get("schemaVersion").and_then(Value::as_u64).unwrap_or(0))?;
    let word = v.pointer("/meta/kind").and_then(Value::as_str);
    if word.and_then(DocKind::from_word) != Some(kind) {
        return Err(MeasureError::Invalid(format!(
            "a {} document, not a {}",
            kind_label(word),
            kind.word()
        )));
    }
    if !v.get(kind.word()).is_some_and(Value::is_object) {
        return Err(MeasureError::Parse(format!("no \"{}\" block", kind.word())));
    }
    serde_json::from_value(v).map_err(|e| MeasureError::Parse(e.to_string()))
}

fn write<T: Serialize>(doc: &T, kind: DocKind) -> Result<String, MeasureError> {
    let text = serde_json::to_string_pretty(doc)
        .map_err(|e| MeasureError::Invalid(format!("not writable: {e}")))?;
    if text.len() > kind.max_bytes() {
        return Err(too_large(text.len(), kind.max_bytes()));
    }
    Ok(text)
}

fn schema(v: u64) -> Result<(), MeasureError> {
    if v == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(MeasureError::Invalid(format!(
            "schemaVersion {v} (this build reads {SCHEMA_VERSION})"
        )))
    }
}

fn text(field: &str, s: &str, max: usize) -> Result<(), MeasureError> {
    // Counting stops one past the bound: a long text costs no more.
    if s.chars().take(max.saturating_add(1)).count() > max {
        Err(MeasureError::Invalid(format!(
            "{field}: more than {max} characters"
        )))
    } else {
        Ok(())
    }
}

/// Empty, or an id the snapshot store could have made: 1 to 64 ASCII digits,
/// lowercase letters and `-`, not starting with `-`.
fn id(field: &str, s: &str) -> Result<(), MeasureError> {
    let ok = s.is_empty()
        || (s.len() <= MAX_ID_LEN
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'-'));
    if ok {
        Ok(())
    } else {
        Err(MeasureError::Invalid(format!("{field}: not a store id")))
    }
}

fn num(field: &str, x: f64, lo: f64, hi: f64) -> Result<(), MeasureError> {
    if x.is_finite() && (lo..=hi).contains(&x) {
        Ok(())
    } else {
        Err(MeasureError::Invalid(format!(
            "{field}: {x} (finite, {lo} to {hi})"
        )))
    }
}

fn opt_num(field: &str, x: Option<f64>, lo: f64, hi: f64) -> Result<(), MeasureError> {
    x.map_or(Ok(()), |x| num(field, x, lo, hi))
}

fn placement(p: &Placement) -> Result<(), MeasureError> {
    let bad = match *p {
        Placement::MultiPosition { count } => !(3..=MAX_POSITIONS).contains(&count),
        Placement::Headphones { reseats, .. } => !(1..=MAX_POSITIONS).contains(&reseats),
        Placement::ListeningPosition | Placement::NearField => false,
    };
    if bad {
        Err(MeasureError::Invalid(format!(
            "measurement.placement: {p:?} (3 to {MAX_POSITIONS} positions, 1 to {MAX_POSITIONS} reseats)"
        )))
    } else {
        Ok(())
    }
}

fn stimulus(s: &Stimulus) -> Result<(), MeasureError> {
    num("measurement.stimulus.f1Hz", s.f1_hz, 1.0, 48_000.0)?;
    num("measurement.stimulus.f2Hz", s.f2_hz, 1.0, 48_000.0)?;
    if s.f2_hz <= s.f1_hz {
        return Err(MeasureError::Invalid(
            "measurement.stimulus: f2Hz is not above f1Hz".into(),
        ));
    }
    let (lo, hi) = match s.kind {
        RunKind::Pink => (1, 60_000),
        RunKind::Walked | RunKind::Summed | RunKind::LiveSweep => (1_000, 15_000),
    };
    let checks = [
        (
            "durationMs",
            (lo..=hi).contains(&s.duration_ms),
            format!("{lo} to {hi}"),
        ),
        ("repeat", (1..=8).contains(&s.repeat), "1 to 8".into()),
        ("gapMs", s.gap_ms <= 10_000, "at most 10000".into()),
        (
            "deviceRateHz",
            matches!(s.device_rate_hz, 44_100 | 48_000 | 96_000),
            "44100, 48000 or 96000".into(),
        ),
    ];
    if let Some((name, _, want)) = checks.into_iter().find(|c| !c.1) {
        return Err(MeasureError::Invalid(format!(
            "measurement.stimulus.{name}: {want}"
        )));
    }
    num("measurement.stimulus.levelDb", s.level_db, -120.0, 0.0)
}

fn capture(c: &CaptureInfo) -> Result<(), MeasureError> {
    if !(8_000..=384_000).contains(&c.rate_hz) {
        return Err(MeasureError::Invalid(format!(
            "measurement.capture.rateHz: {} (8000 to 384000)",
            c.rate_hz
        )));
    }
    num("measurement.capture.clockRatio", c.clock_ratio, 0.99, 1.01)?;
    opt_num(
        "measurement.capture.maxResidualUs",
        c.max_residual_us,
        0.0,
        1e9,
    )
}

fn chain(c: &ChannelChain) -> Result<(), MeasureError> {
    for (name, bank) in [("peq", &c.peq), ("crossover", &c.crossover)] {
        if bank.len() > MAX_BANDS {
            return Err(MeasureError::Invalid(format!(
                "measurement.outputs.chain.{name}: {} bands (at most {MAX_BANDS})",
                bank.len()
            )));
        }
        if !bank.iter().all(band_finite) {
            return Err(MeasureError::Invalid(format!(
                "measurement.outputs.chain.{name}: a band is not finite"
            )));
        }
    }
    let field = "measurement.outputs.chain";
    num(field, f64::from(c.gain_db), -200.0, 200.0)?;
    num(field, f64::from(c.delay_ms), -1_000.0, 1_000.0)?;
    num(field, f64::from(c.feed.gain_db), -200.0, 200.0)
}

fn band_finite(b: &BandSpec) -> bool {
    [b.freq, b.q, b.gain, b.qp].iter().all(|x| x.is_finite())
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5mdoc;
