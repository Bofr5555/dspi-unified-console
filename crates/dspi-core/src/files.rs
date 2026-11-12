//! Configuration documents and filter files (WP C13).
//!
//! Two kinds of file carry a tuning between devices and applications:
//!
//! - **The configuration document** (`.dspipreset`, JSON, schema 1): the whole
//!   device. [`DspiCore::config_export`] writes it from the mirror,
//!   [`config_inspect`] describes one before it is applied, and
//!   [`DspiCore::config_import`] applies it as one bulk write. The format is the
//!   macOS Console's (`PresetDocument.swift`) and the Terminal's
//!   (`dspi_session::preset_file`), written down in
//!   `docs/terminal/preset-format.md`.
//! - **The filter file** (text): every channel's EQ and crossover bands. This
//!   build writes the Console's format 2; it reads that, the Windows Console's
//!   name-keyed sections and REW or AutoEQ lists (`FilterFile.swift`).
//!
//! Two rules hold throughout. An export says only what the device's wire
//! format carries (`dspi_proto::wire_compat::features`), so a document from an
//! older device has no key its firmware lacks. An import changes only what the
//! file holds a readable value for: everything else keeps the device's value,
//! and the report names it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use dspi_proto::FilterType;
use dspi_proto::ext::PairMap;
use dspi_proto::generated::{limiter, tube};
use dspi_proto::wire::INPUT_CONFIG_FIELDS;
use dspi_proto::wire_compat::{self, WireRead, feat};
use dspi_proto::{Platform, xover};
use dspi_session::preset_file::{self, ChannelRef};
use serde_json::{Map, Value, json};

use crate::connect::{LOCK_REASON, apply_all_params_locked, refresh_params_locked};
use crate::consts::*;
use crate::ext::{crossfeed_pairs, set_pair_map_locked};
use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Inner, Result, state};

/// The largest file this module reads: far above any real document (a full
/// RP2350 export is about 100 kB), low enough that a wrong file picked by
/// mistake is refused before it is parsed.
pub const MAX_FILE_BYTES: usize = 4 << 20;

/// The document schema this build reads and writes.
const SCHEMA_VERSION: i64 = 1;

/// The filter file format this build writes (`# Format: 2`).
const FILTER_FORMAT: u32 = 2;

/// The last line of every import report.
const NOT_STORED: &str = "These changes are live but not yet stored on the device. \
                          Save them to a preset slot to keep them.";

/// A filter import found a chosen section without `Filter` lines.
const NO_EQ_BANK: &str =
    "A section with no Filter lines left that channel's EQ bands as they were.";

/// A filter import found a chosen output section without `Xover` lines.
const NO_XOVER_BANK: &str =
    "A section with no Xover lines left that output's crossover bands as they were.";

/// Refusal of a file over [`MAX_FILE_BYTES`].
const TOO_LARGE: &str = "This file is too large to be a DSPi file (the limit is 4 MB).";

/// Missing channels named in a report before the rest are counted.
const MISSING_NAMED: usize = 8;

// ═════════════════════════════════════════════════════════════════════
// Reading JSON leniently
// ═════════════════════════════════════════════════════════════════════

/// A value that is there: absent and `null` both mean "the file does not say".
fn given(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

/// Reads a document's values. A value of the wrong type counts as absent, and
/// is remembered so the report can say that something could not be read.
#[derive(Default)]
struct Reader {
    unreadable: bool,
}

impl Reader {
    fn wrong<T>(&mut self) -> Option<T> {
        self.unreadable = true;
        None
    }

    fn real(&mut self, v: Option<&Value>) -> Option<f32> {
        let v = given(v)?;
        match v.as_f64().map(|x| x as f32) {
            Some(x) if x.is_finite() => Some(x),
            _ => self.wrong(),
        }
    }

    fn switch(&mut self, v: Option<&Value>) -> Option<bool> {
        let v = given(v)?;
        match v.as_bool() {
            Some(b) => Some(b),
            None => self.wrong(),
        }
    }

    /// A whole number, held to `lo..=hi`: a value past the range takes the
    /// nearest end rather than wrapping. A fraction is the wrong type.
    fn whole(&mut self, v: Option<&Value>, lo: i64, hi: i64) -> Option<i64> {
        let v = given(v)?;
        let n = v
            .as_i64()
            .or_else(|| v.as_u64().map(|_| i64::MAX))
            .or_else(|| {
                v.as_f64()
                    .filter(|f| f.is_finite() && f.fract() == 0.0)
                    .map(|f| f.clamp(-9.0e18, 9.0e18) as i64)
            });
        match n {
            Some(n) => Some(n.clamp(lo, hi)),
            None => self.wrong(),
        }
    }

    fn byte(&mut self, v: Option<&Value>) -> Option<u8> {
        self.whole(v, 0, 255).and_then(|n| u8::try_from(n).ok())
    }

    fn word(&mut self, v: Option<&Value>) -> Option<u16> {
        self.whole(v, 0, 65_535).and_then(|n| u16::try_from(n).ok())
    }

    fn text(&mut self, v: Option<&Value>) -> Option<String> {
        let v = given(v)?;
        match v.as_str() {
            Some(s) => Some(s.to_string()),
            None => self.wrong(),
        }
    }

    fn object<'v>(&mut self, v: Option<&'v Value>) -> Option<&'v Map<String, Value>> {
        let v = given(v)?;
        match v.as_object() {
            Some(o) => Some(o),
            None => self.wrong(),
        }
    }

    fn list<'v>(&mut self, v: Option<&'v Value>) -> Option<&'v [Value]> {
        let v = given(v)?;
        match v.as_array() {
            Some(a) => Some(a.as_slice()),
            None => self.wrong(),
        }
    }
}

// ═════════════════════════════════════════════════════════════════════
// The scalar settings, one table per block
// ═════════════════════════════════════════════════════════════════════

/// How an entry of a parameter array (`upmix`, `subharm`, `tube`) is written
/// in a document.
#[derive(Clone, Copy)]
enum Shape {
    /// A number as it is.
    Level,
    /// `true` / `false`, held as 1 / 0.
    Flag,
    /// A whole number up to the bound.
    Count(u16),
}

/// Where a document key lives in the mirror.
#[derive(Clone, Copy)]
enum Slot {
    Real(fn(&mut DspState) -> &mut f32),
    Switch(fn(&mut DspState) -> &mut bool),
    Byte(fn(&mut DspState) -> &mut u8),
    Word(fn(&mut DspState) -> &mut u16),
    Param(fn(&mut DspState) -> &mut Vec<f32>, usize, Shape),
}

/// One key of a block: its name, the `feat::*` bit of the wire format that
/// first carried it (0: every format), and its place in the mirror. Export
/// and import read the same table, so the two cannot disagree on a key.
#[derive(Clone, Copy)]
struct Field {
    key: &'static str,
    since: u32,
    slot: Slot,
}

const fn key(key: &'static str, since: u32, slot: Slot) -> Field {
    Field { key, since, slot }
}

fn upmix_params(s: &mut DspState) -> &mut Vec<f32> {
    &mut s.upmix
}

fn subharm_params(s: &mut DspState) -> &mut Vec<f32> {
    &mut s.subharm
}

fn tube_params(s: &mut DspState) -> &mut Vec<f32> {
    &mut s.tube
}

fn up(id: u8, shape: Shape) -> Slot {
    Slot::Param(upmix_params, usize::from(id), shape)
}

fn sub(id: u8, shape: Shape) -> Slot {
    Slot::Param(subharm_params, usize::from(id), shape)
}

fn tb(id: u16, shape: Shape) -> Slot {
    Slot::Param(tube_params, usize::from(id), shape)
}

fn global_fields() -> Vec<Field> {
    vec![
        key("bypass", 0, Slot::Switch(|s| &mut s.bypass)),
        key("inputSource", 0, Slot::Byte(|s| &mut s.input_source)),
        key(
            "lgSoundSyncEnabled",
            0,
            Slot::Switch(|s| &mut s.lg_sound_sync_enabled),
        ),
    ]
}

fn volume_fields() -> Vec<Field> {
    vec![
        key("masterVolumeDb", 0, Slot::Real(|s| &mut s.master_volume_db)),
        key("userVolumeDb", 0, Slot::Real(|s| &mut s.user_volume_db)),
    ]
}

fn loudness_fields() -> Vec<Field> {
    vec![
        key("enabled", 0, Slot::Switch(|s| &mut s.loudness_enabled)),
        key("refSpl", 0, Slot::Real(|s| &mut s.loudness_ref_spl)),
        key("intensityPct", 0, Slot::Real(|s| &mut s.loudness_intensity)),
        key(
            "outputMask",
            feat::LOUDNESS_MASK,
            Slot::Word(|s| &mut s.loudness_output_mask),
        ),
    ]
}

fn crossfeed_fields() -> Vec<Field> {
    vec![
        key("enabled", 0, Slot::Switch(|s| &mut s.crossfeed_enabled)),
        key("preset", 0, Slot::Byte(|s| &mut s.crossfeed_preset)),
        key("freqHz", 0, Slot::Real(|s| &mut s.crossfeed_freq)),
        key("feedDb", 0, Slot::Real(|s| &mut s.crossfeed_feed)),
        key("itd", 0, Slot::Switch(|s| &mut s.crossfeed_itd)),
        key(
            "outputPairMask",
            feat::CROSSFEED_MASK,
            Slot::Byte(|s| &mut s.crossfeed_output_pair_mask),
        ),
    ]
}

fn leveller_fields() -> Vec<Field> {
    vec![
        key("enabled", 0, Slot::Switch(|s| &mut s.leveller_enabled)),
        key("speed", 0, Slot::Byte(|s| &mut s.leveller_speed)),
        key("lookahead", 0, Slot::Switch(|s| &mut s.leveller_lookahead)),
        key("amountPct", 0, Slot::Real(|s| &mut s.leveller_amount)),
        key("maxGainDb", 0, Slot::Real(|s| &mut s.leveller_max_gain_db)),
        key("gateDb", 0, Slot::Real(|s| &mut s.leveller_gate_db)),
        key(
            "detectorMask",
            feat::LEVELLER_MASKS,
            Slot::Byte(|s| &mut s.leveller_detector_mask),
        ),
        key(
            "applyMask",
            feat::LEVELLER_MASKS,
            Slot::Byte(|s| &mut s.leveller_apply_mask),
        ),
    ]
}

fn psybass_fields() -> Vec<Field> {
    vec![
        key("enabled", 0, Slot::Switch(|s| &mut s.psybass_enabled)),
        key("cutoffHz", 0, Slot::Real(|s| &mut s.psybass_cutoff_hz)),
        key(
            "harmonicsDb",
            0,
            Slot::Real(|s| &mut s.psybass_harmonics_db),
        ),
        key("driveDb", 0, Slot::Real(|s| &mut s.psybass_drive_db)),
        key(
            "characterPct",
            0,
            Slot::Real(|s| &mut s.psybass_character_pct),
        ),
        key("originalDb", 0, Slot::Real(|s| &mut s.psybass_original_db)),
        key("outputMask", 0, Slot::Word(|s| &mut s.psybass_output_mask)),
    ]
}

fn upmix_fields() -> Vec<Field> {
    use Shape::*;
    vec![
        key("enabled", 0, up(UPMIX_PARAM_ENABLED, Flag)),
        key("centerMode", 0, up(UPMIX_PARAM_CENTER_MODE, Count(255))),
        key("surroundMode", 0, up(UPMIX_PARAM_SURROUND_MODE, Count(255))),
        key("strengthPct", 0, up(UPMIX_PARAM_STRENGTH, Level)),
        key("centerWidthPct", 0, up(UPMIX_PARAM_WIDTH, Level)),
        key("thresholdPct", 0, up(UPMIX_PARAM_THRESHOLD, Level)),
        key("attackMs", 0, up(UPMIX_PARAM_ATTACK, Level)),
        key("releaseMs", 0, up(UPMIX_PARAM_RELEASE, Level)),
        key("detectorHpfHz", 0, up(UPMIX_PARAM_DETECTOR_HPF, Level)),
        key("surroundDelayMs", 0, up(UPMIX_PARAM_SURROUND_DELAY, Level)),
        key("surroundHpfHz", 0, up(UPMIX_PARAM_SURROUND_HPF, Level)),
        key("surroundLpfHz", 0, up(UPMIX_PARAM_SURROUND_LPF, Level)),
        key("decorrPct", 0, up(UPMIX_PARAM_DECORRELATION, Level)),
        key(
            "presenceDb",
            feat::UPMIX_PRESENCE,
            up(UPMIX_PARAM_PRESENCE, Level),
        ),
    ]
}

fn subharm_fields() -> Vec<Field> {
    use Shape::*;
    let ext = feat::SUBHARM_EXT;
    vec![
        key("enabled", 0, sub(SUBHARM_PARAM_ENABLED, Flag)),
        key("outputMask", 0, sub(SUBHARM_PARAM_MASK, Count(u16::MAX))),
        key("lowDb", 0, sub(SUBHARM_PARAM_LOW, Level)),
        key("highDb", 0, sub(SUBHARM_PARAM_HIGH, Level)),
        key("boostDb", 0, sub(SUBHARM_PARAM_BOOST, Level)),
        key("topDb", ext, sub(SUBHARM_PARAM_TOP, Level)),
        key("selectMode", ext, sub(SUBHARM_PARAM_SELECT, Count(255))),
        key("selectDepthPct", ext, sub(SUBHARM_PARAM_DEPTH, Level)),
        key("selectHoldMs", ext, sub(SUBHARM_PARAM_HOLD, Level)),
        key("ceilingDb", ext, sub(SUBHARM_PARAM_CEILING, Level)),
        key("linkPairs", ext, sub(SUBHARM_PARAM_LINK, Flag)),
    ]
}

fn tube_fields() -> Vec<Field> {
    use Shape::*;
    vec![
        key("enabled", 0, tb(tube::TUBE_PARAM_ENABLED, Flag)),
        key(
            "outputMask",
            0,
            tb(tube::TUBE_PARAM_OUTPUT_MASK, Count(u16::MAX)),
        ),
        key("tubeType", 0, tb(tube::TUBE_PARAM_TUBE_TYPE, Count(255))),
        key("driveDb", 0, tb(tube::TUBE_PARAM_DRIVE_DB, Level)),
        key("biasPct", 0, tb(tube::TUBE_PARAM_BIAS_PCT, Level)),
        key("asymDb", 0, tb(tube::TUBE_PARAM_ASYM_DB, Level)),
        key("hardnessPct", 0, tb(tube::TUBE_PARAM_HARDNESS_PCT, Level)),
        key("sagPct", 0, tb(tube::TUBE_PARAM_SAG_PCT, Level)),
        key("rectifier", 0, tb(tube::TUBE_PARAM_RECTIFIER, Count(255))),
        key("xfmrEnabled", 0, tb(tube::TUBE_PARAM_XFMR_ENABLED, Flag)),
        key("xfmrDamping", 0, tb(tube::TUBE_PARAM_XFMR_DAMPING, Level)),
        key("xfmrResHz", 0, tb(tube::TUBE_PARAM_XFMR_RES_HZ, Level)),
        key("mixPct", 0, tb(tube::TUBE_PARAM_MIX_PCT, Level)),
        key("trimDb", 0, tb(tube::TUBE_PARAM_TRIM_DB, Level)),
    ]
}

/// The `io` keys that are one value each. The lists, the two coded values
/// (MCK multiplier, I2S rate), the S/PDIF mask and the DAC mute are handled
/// by hand.
fn io_fields() -> Vec<Field> {
    let clocks = feat::I2S_CLOCK_MODE;
    vec![
        key("i2sBckPin", 0, Slot::Byte(|s| &mut s.i2s_bck_pin)),
        key("mckEnabled", 0, Slot::Switch(|s| &mut s.mck_enabled)),
        key("mckPin", 0, Slot::Byte(|s| &mut s.mck_pin)),
        key(
            "i2sInputChannels",
            0,
            Slot::Byte(|s| &mut s.i2s_input_channels),
        ),
        key(
            "i2sClockMode",
            clocks,
            Slot::Byte(|s| &mut s.i2s_clock_mode),
        ),
        key(
            "i2sClockPinMode",
            clocks,
            Slot::Byte(|s| &mut s.i2s_clock_pin_mode),
        ),
        key(
            "i2sBckPinSlave",
            clocks,
            Slot::Byte(|s| &mut s.i2s_bck_pin_slave),
        ),
        key(
            "adatEnabled",
            feat::ADAT_OUT,
            Slot::Switch(|s| &mut s.adat_out_enabled),
        ),
        key(
            "adatPin",
            feat::ADAT_OUT,
            Slot::Byte(|s| &mut s.adat_out_pin),
        ),
        key(
            "adatInputEnabled",
            feat::ADAT_IN,
            Slot::Switch(|s| &mut s.adat_input_enabled),
        ),
        key(
            "adatInputPin",
            feat::ADAT_IN,
            Slot::Byte(|s| &mut s.adat_input_pin),
        ),
        key(
            "adatInputClockMode",
            feat::ADAT_IN,
            Slot::Byte(|s| &mut s.adat_input_clock_mode),
        ),
    ]
}

/// One output's limiter, as `channels[].limiter`.
fn limiter_fields(o: usize) -> [(&'static str, LimiterKey, usize); 4] {
    [
        ("enabled", LimiterKey::Enabled, o),
        ("thresholdDb", LimiterKey::Threshold, o),
        ("releaseMs", LimiterKey::Release, o),
        ("linkGroup", LimiterKey::LinkGroup, o),
    ]
}

#[derive(Clone, Copy)]
enum LimiterKey {
    Enabled,
    Threshold,
    Release,
    LinkGroup,
}

/// A slot's value as the document writes it; `None` past a parameter array.
fn slot_value(s: &mut DspState, slot: Slot) -> Option<Value> {
    Some(match slot {
        Slot::Real(f) => json!(*f(s)),
        Slot::Switch(f) => json!(*f(s)),
        Slot::Byte(f) => json!(*f(s)),
        Slot::Word(f) => json!(*f(s)),
        Slot::Param(f, i, shape) => {
            let v = *f(s).get(i)?;
            match shape {
                Shape::Level => json!(v),
                Shape::Flag => json!(v != 0.0),
                Shape::Count(_) => json!(v.round() as i64),
            }
        }
    })
}

/// Set a slot from a document value; whether it was readable.
fn put_slot(read: &mut Reader, s: &mut DspState, slot: Slot, v: Option<&Value>) -> bool {
    match slot {
        Slot::Real(f) => read.real(v).map(|x| *f(s) = x).is_some(),
        Slot::Switch(f) => read.switch(v).map(|x| *f(s) = x).is_some(),
        Slot::Byte(f) => read.byte(v).map(|x| *f(s) = x).is_some(),
        Slot::Word(f) => read.word(v).map(|x| *f(s) = x).is_some(),
        Slot::Param(f, i, shape) => {
            let x = match shape {
                Shape::Level => read.real(v),
                Shape::Flag => read.switch(v).map(|b| if b { 1.0 } else { 0.0 }),
                Shape::Count(max) => read.whole(v, 0, i64::from(max)).map(|n| n as f32),
            };
            match (x, f(s).get_mut(i)) {
                (Some(x), Some(p)) => {
                    *p = x;
                    true
                }
                _ => false,
            }
        }
    }
}

/// The keys of `fields` the wire format carries (`has`), as a JSON object.
fn block_value(
    s: &mut DspState,
    fields: &[Field],
    has: &dyn Fn(u32) -> bool,
) -> Map<String, Value> {
    let mut out = Map::new();
    for f in fields.iter().filter(|f| has(f.since)) {
        if let Some(v) = slot_value(s, f.slot) {
            out.insert(f.key.to_string(), v);
        }
    }
    out
}

// ═════════════════════════════════════════════════════════════════════
// Export
// ═════════════════════════════════════════════════════════════════════

/// A flat band: what a bank's unused slots hold.
fn flat() -> FilterParams {
    FilterParams {
        filter_type: 0,
        bypass: false,
        freq: 1000.0,
        q: DEFAULT_Q,
        gain: 0.0,
        qp: DEFAULT_Q,
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since the Unix epoch.
// Days since 1970 to a civil date (the proleptic Gregorian calendar, in
// 400-year eras). Every quantity is bounded by `secs / 86400`, far from any
// overflow.
#[allow(clippy::arithmetic_side_effects)]
fn utc_text(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let of_day = secs % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let m = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * m + 2) / 5 + 1;
    let month = if m < 10 { m + 3 } else { m - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day / 60 % 60,
        of_day % 60
    )
}

/// Seconds since the Unix epoch, by the core's clock.
fn unix_now() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether the image says the device set the input-config byte `name[at]`
/// (0 is "unset" there). Without a V32 image to look in, it did.
fn image_sets(s: &DspState, name: &str, at: usize) -> bool {
    let Some(&(_, field, _)) = INPUT_CONFIG_FIELDS.iter().find(|(n, _, _)| *n == name) else {
        return true;
    };
    let Some(base) = dspi_proto::generated::SECTIONS
        .iter()
        .find(|(n, _, _)| *n == "input_config")
        .map(|(_, o, _)| *o)
    else {
        return true;
    };
    if s.bulk_raw.len() != dspi_proto::generated::BULK_SIZE {
        return true;
    }
    base.checked_add(field)
        .and_then(|i| i.checked_add(at))
        .and_then(|i| s.bulk_raw.get(i))
        .is_none_or(|b| *b != 0)
}

/// A pin list: the first entry, then the extension entries the image set,
/// stopping at the first it left unset.
fn pin_list(s: &DspState, pins: &[u8], ext_field: &str, ext_count: usize) -> Vec<u8> {
    let mut out: Vec<u8> = pins.iter().take(1).copied().collect();
    for k in 0..ext_count {
        match pins.get(k.saturating_add(1)) {
            Some(&p) if image_sets(s, ext_field, k) => out.push(p),
            _ => break,
        }
    }
    out
}

fn band_value(b: &FilterParams, with_qp: bool) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), json!(b.filter_type));
    o.insert("freqHz".into(), json!(b.freq));
    o.insert("q".into(), json!(b.q));
    o.insert("gain".into(), json!(b.gain));
    o.insert("bypass".into(), json!(b.bypass));
    if with_qp {
        o.insert("qp".into(), json!(b.qp));
    }
    Value::Object(o)
}

fn bank_value(bank: Option<&Vec<FilterParams>>, width: usize, with_qp: bool) -> Value {
    Value::Array(
        bank.map(|b| {
            b.iter()
                .take(width)
                .map(|p| band_value(p, with_qp))
                .collect()
        })
        .unwrap_or_default(),
    )
}

/// The mirror `s` as a document. `links` is the app's input-pair link mask;
/// `pair_map` the crossfeed pair map the device reports over the extension
/// channel, written only when there is one.
fn export_config(
    s: &DspState,
    name: &str,
    app_version: &str,
    links: u8,
    pair_map: Option<[u8; 4]>,
) -> String {
    let carried = wire_compat::features(s.format_version);
    let has = |bit: u32| bit == 0 || carried & bit != 0;
    let mut m = s.clone();
    let n_in = s.num_input_channels;
    let n_out = s.num_output_channels;
    let rp2350 = Platform::from_id(s.platform_id) == Platform::Rp2350;
    let with_qp = has(feat::LINKWITZ);

    let mut doc = Map::new();
    doc.insert("schemaVersion".into(), json!(SCHEMA_VERSION));
    doc.insert(
        "meta".into(),
        json!({
            "name": name,
            "savedUtc": utc_text(unix_now()),
            "appVersion": app_version,
            "platform": Platform::from_id(s.platform_id).name(),
            "firmwareVersion": format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch),
            "wireFormatVersion": s.format_version,
            "inputChannelCount": n_in,
            "outputChannelCount": n_out,
            "masterVolumeMode": s.master_volume_mode,
            "outputConfigMode": s.output_config_mode,
        }),
    );

    let mut global = block_value(&mut m, &global_fields(), &has);
    global.extend(block_value(&mut m, &volume_fields(), &has));
    global.insert(
        "inputPreampsDb".into(),
        json!(
            s.input_preamp_db
                .iter()
                .take(usize::from(n_in))
                .collect::<Vec<_>>()
        ),
    );
    global.insert(
        "inputPairLinked".into(),
        json!((0..4u8).map(|p| (links >> p) & 1 == 1).collect::<Vec<_>>()),
    );
    doc.insert("global".into(), Value::Object(global));

    doc.insert(
        "loudness".into(),
        Value::Object(block_value(&mut m, &loudness_fields(), &has)),
    );
    let mut crossfeed = block_value(&mut m, &crossfeed_fields(), &has);
    if let Some(map) = pair_map {
        crossfeed.insert("pairMap".into(), json!(map));
    }
    doc.insert("crossfeed".into(), Value::Object(crossfeed));
    doc.insert(
        "leveller".into(),
        Value::Object(block_value(&mut m, &leveller_fields(), &has)),
    );
    let optional: [(&str, u32, bool, Vec<Field>); 4] = [
        ("psybass", feat::PSYBASS, true, psybass_fields()),
        ("upmix", feat::UPMIX, rp2350, upmix_fields()),
        ("subharm", feat::SUBHARM, true, subharm_fields()),
        ("tube", feat::TUBE, true, tube_fields()),
    ];
    for (block, bit, platform, fields) in optional {
        if has(bit) && platform {
            doc.insert(
                block.into(),
                Value::Object(block_value(&mut m, &fields, &has)),
            );
        }
    }

    let mut channels = Vec::new();
    for i in 0..n_in {
        let wire = usize::from(i);
        channels.push(json!({
            "channelId": preset_file::channel_id::for_input(i),
            "eqChannel": i,
            "inputIndex": i,
            "isOutput": false,
            "name": s.channel_names.get(wire).cloned().unwrap_or_default(),
            "delayMs": s.channel_delays.get(wire).copied().unwrap_or(0.0),
            "gainDb": 0.0,
            "muted": false,
            "enabled": true,
            "eq": bank_value(s.filters.get(wire), BANDS_PER_CHANNEL, with_qp),
            "crossover": [],
        }));
    }
    for o in 0..n_out {
        let at = usize::from(o);
        let wire = usize::from(n_in).saturating_add(at);
        let mut c = json!({
            "channelId": preset_file::channel_id::for_output(o),
            "eqChannel": wire,
            "outputIndex": o,
            "isOutput": true,
            "name": s.channel_names.get(wire).cloned().unwrap_or_default(),
            "delayMs": s.channel_delays.get(wire).copied().unwrap_or(0.0),
            "gainDb": s.output_gain_db.get(at).copied().unwrap_or(0.0),
            "muted": s.output_muted.get(at).copied().unwrap_or(false),
            "enabled": s.output_enabled.get(at).copied().unwrap_or(false),
            "outputDelayMs": s.output_delay_ms.get(at).copied().unwrap_or(0.0),
            "eq": bank_value(s.filters.get(wire), BANDS_PER_CHANNEL, with_qp),
            "crossover": bank_value(s.xover.get(wire), MAX_XOVER_BANDS, with_qp),
        });
        if has(feat::LIMITER)
            && let Some(obj) = c.as_object_mut()
        {
            obj.insert(
                "limiter".into(),
                json!({
                    "enabled": s.limiter_enabled.get(at).copied().unwrap_or(false),
                    "thresholdDb": s.limiter_threshold_db.get(at).copied().unwrap_or(0.0),
                    "releaseMs": s.limiter_release_ms.get(at).copied().unwrap_or(0.0),
                    "linkGroup": s.limiter_link_group.get(at).copied().unwrap_or(0),
                }),
            );
        }
        channels.push(c);
    }
    doc.insert("channels".into(), Value::Array(channels));

    let mut matrix = Vec::new();
    for i in 0..usize::from(n_in) {
        for o in 0..usize::from(n_out) {
            let at = |m: &Vec<Vec<bool>>| m.get(i).and_then(|r| r.get(o)).copied().unwrap_or(false);
            matrix.push(json!({
                "input": i,
                "output": o,
                "enabled": at(&s.matrix_routing),
                "invert": at(&s.matrix_invert),
                "gainDb": s.matrix_gain.get(i).and_then(|r| r.get(o)).copied().unwrap_or(0.0),
            }));
        }
    }
    doc.insert("matrix".into(), Value::Array(matrix));
    doc.insert("io".into(), Value::Object(io_value(s, &mut m, &has)));

    serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default()
}

fn io_value(s: &DspState, m: &mut DspState, has: &dyn Fn(u32) -> bool) -> Map<String, Value> {
    let mut io = block_value(m, &io_fields(), has);
    let pins: Vec<u8> = s
        .output_pins
        .iter()
        .take(usize::from(s.num_pin_outputs))
        .copied()
        .collect();
    io.insert("outputPins".into(), json!(pins));
    io.insert("outputSlotTypes".into(), json!(s.output_types));
    io.insert(
        "mckMultiplier".into(),
        json!(if s.mck_multiplier == 1 { 256 } else { 128 }),
    );
    io.insert(
        "i2sInputRateHz".into(),
        json!(match s.i2s_input_rate {
            0 => 44_100,
            2 => 96_000,
            _ => 48_000,
        }),
    );
    io.insert(
        "i2sRxPins".into(),
        json!(pin_list(s, &s.i2s_rx_pins, "i2s_rx_pin_ext", 3)),
    );
    // S/PDIF inputs 2 and 3 came with V21, the fourth with V28 in a key of its
    // own (the shared schema's list is three long).
    let spdif_ext = if has(feat::I2S_CLOCK_MODE) { 2 } else { 0 };
    io.insert(
        "spdifRxPins".into(),
        json!(pin_list(s, &s.spdif_rx_pins, "spdif_rx_pin_ext", spdif_ext)),
    );
    if has(feat::SPDIF_IN4)
        && image_sets(s, "spdif_rx_pin_ext", 2)
        && let Some(p) = s.spdif_rx_pins.get(3)
    {
        io.insert("spdifRxPin4".into(), json!(p));
    }
    if has(feat::I2S_CLOCK_MODE) {
        io.insert(
            "spdifEnabledExt".into(),
            json!((s.spdif_inputs_enabled >> 1) & 0x07),
        );
    }
    io.insert(
        "dacHwMute".into(),
        if s.dac_mute_supported {
            json!({
                "enabled": s.dac_mute_enabled,
                "activeLow": s.dac_mute_active_low,
                "pin": s.dac_mute_pin,
                "holdMs": s.dac_mute_hold_ms,
                "releaseMs": s.dac_mute_release_ms,
            })
        } else {
            Value::Null
        },
    );
    io
}

// ═════════════════════════════════════════════════════════════════════
// Import: reading a document onto a state
// ═════════════════════════════════════════════════════════════════════

/// Check a document's text and parse it: JSON, an object, a schema this build
/// reads, and at least one channel. The error is a sentence for the user.
fn parse_config(text: &str) -> std::result::Result<Value, String> {
    if text.len() > MAX_FILE_BYTES {
        return Err(TOO_LARGE.into());
    }
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Err("This file is empty.".into());
    }
    let doc: Value = serde_json::from_str(text)
        .map_err(|e| format!("This file is not a DSPi configuration: {e}."))?;
    let Some(top) = doc.as_object() else {
        return Err("This file is not a DSPi configuration: it holds no settings.".into());
    };
    let mut read = Reader::default();
    match read.whole(top.get("schemaVersion"), i64::MIN, i64::MAX) {
        Some(v) if v > SCHEMA_VERSION => {
            return Err(format!(
                "This file was written by a newer version (format {v}; this build reads \
                 format {SCHEMA_VERSION})."
            ));
        }
        Some(v) if v >= 1 => {}
        _ => {
            return Err("This file is not a DSPi configuration: it has no schema version.".into());
        }
    }
    match top.get("channels").and_then(Value::as_array) {
        Some(c) if !c.is_empty() => Ok(doc),
        _ => Err("This file holds no channel settings.".into()),
    }
}

/// Channels a document names that the device does not have.
#[derive(Debug, Default)]
struct Missing {
    names: Vec<String>,
}

impl Missing {
    fn add(&mut self, name: String) {
        self.names.push(name);
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The report's line: the first few by name, the rest counted.
    fn line(&self) -> Option<String> {
        if self.names.is_empty() {
            return None;
        }
        let named = self
            .names
            .iter()
            .take(MISSING_NAMED)
            .cloned()
            .collect::<Vec<_>>();
        let more = self.names.len().saturating_sub(named.len());
        let mut line = format!("Not present on this device: {}", named.join(", "));
        if more > 0 {
            line.push_str(&format!(", and {more} more"));
        }
        Some(line)
    }
}

/// Reasons a part of a file was not applied, each once.
#[derive(Debug, Default)]
struct Skips {
    reasons: Vec<String>,
}

impl Skips {
    fn add(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }

    fn is_empty(&self) -> bool {
        self.reasons.is_empty()
    }
}

/// A document read onto a device's state: the state to write, and what the
/// report says about it.
#[derive(Debug)]
struct Built {
    state: DspState,
    channels: usize,
    bands: usize,
    xover: usize,
    crosspoints: usize,
    missing: Missing,
    skipped: Skips,
    /// Settings the document holds no readable value for, by JSON key.
    left_alone: Vec<String>,
    /// "<channel> (<ms> ms)" for each delay held to the device's line.
    shortened: Vec<String>,
    warnings: Vec<String>,
    /// A value had the wrong type (and was left alone).
    unreadable: bool,
    /// `global.inputPairLinked` as a mask, when the document has it.
    links: Option<u8>,
    /// The crossfeed pair map the document asks for, as applied.
    pair_map: Option<[u8; 4]>,
}

struct Importer<'d> {
    doc: &'d Map<String, Value>,
    options: u32,
    read: Reader,
    out: Built,
}

/// Read document `doc` onto `base` (the device's state) with the
/// `IMPORT_*` `options`. Pure: nothing is sent.
fn build_config(base: &DspState, doc: &Value, options: u32) -> Built {
    let empty = Map::new();
    let mut im = Importer {
        doc: doc.as_object().unwrap_or(&empty),
        options,
        read: Reader::default(),
        out: Built {
            state: base.clone(),
            channels: 0,
            bands: 0,
            xover: 0,
            crosspoints: 0,
            missing: Missing::default(),
            skipped: Skips::default(),
            left_alone: Vec::new(),
            shortened: Vec::new(),
            warnings: Vec::new(),
            unreadable: false,
            links: None,
            pair_map: None,
        },
    };
    let placed = im.place_channels();
    let inputs = placed
        .keys()
        .filter(|r| matches!(r, ChannelRef::Input(_)))
        .count();
    im.global(inputs);
    for (&place, entry) in &placed {
        im.channel(place, entry);
    }
    im.out.channels = placed.len();
    im.matrix();
    im.feature_blocks();
    if options & IMPORT_HARDWARE != 0 {
        im.io();
    }
    im.old_format();
    im.out.unreadable = im.read.unreadable;
    im.out
}

impl<'d> Importer<'d> {
    fn leave(&mut self, key: impl Into<String>) {
        let key = key.into();
        if !self.out.left_alone.contains(&key) {
            self.out.left_alone.push(key);
        }
    }

    fn inputs(&self) -> u8 {
        self.out.state.num_input_channels
    }

    fn outputs(&self) -> u8 {
        self.out.state.num_output_channels
    }

    /// Apply the `fields` of `obj`; name each one it lacks as `prefix.key`.
    /// Returns the keys applied.
    fn fields(
        &mut self,
        obj: Option<&Map<String, Value>>,
        prefix: &str,
        fields: &[Field],
    ) -> Vec<&'static str> {
        let mut applied = Vec::new();
        for f in fields {
            let v = obj.and_then(|o| o.get(f.key));
            if put_slot(&mut self.read, &mut self.out.state, f.slot, v) {
                applied.push(f.key);
            } else {
                self.leave(format!("{prefix}.{}", f.key));
            }
        }
        applied
    }

    /// A settings block: with no readable key at all it is named once.
    fn block(&mut self, name: &str, fields: &[Field]) {
        let Some(obj) = self.read.object(self.doc.get(name)) else {
            self.leave(name);
            return;
        };
        let before = self.out.left_alone.len();
        let applied = self.fields(Some(obj), name, fields);
        if applied.is_empty() {
            self.out.left_alone.truncate(before);
            self.leave(name);
        }
    }

    /// Where each channel entry lands on this device. A duplicate takes the
    /// last entry; one for a channel the device lacks is named as missing.
    fn place_channels(&mut self) -> BTreeMap<ChannelRef, &'d Map<String, Value>> {
        let doc = self.doc;
        let mut placed = BTreeMap::new();
        for entry in doc
            .get("channels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(e) = self.read.object(Some(entry)) else {
                continue;
            };
            let place = self.placement(e).filter(|r| match *r {
                ChannelRef::Input(i) => i < self.inputs(),
                ChannelRef::Output(o) => o < self.outputs(),
            });
            match place {
                Some(r) => {
                    placed.insert(r, e);
                }
                None => {
                    let name = e
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| match e.get("channelId").and_then(Value::as_i64) {
                            Some(id) => format!("channel {id}"),
                            None => "a channel".into(),
                        });
                    self.out.missing.add(name);
                }
            }
        }
        placed
    }

    /// The Console's rule: an entry's own `inputIndex` / `outputIndex`, else
    /// its `channelId` through the shared id table. An index that is no
    /// channel number (negative, past 255) places it nowhere.
    fn placement(&mut self, e: &Map<String, Value>) -> Option<ChannelRef> {
        let output = self.read.switch(e.get("isOutput")).unwrap_or(false);
        let own = if output { "outputIndex" } else { "inputIndex" };
        if given(e.get(own)).is_some() {
            let n = self.read.whole(e.get(own), -1, 256)?;
            let n = u8::try_from(n).ok()?;
            return Some(if output {
                ChannelRef::Output(n)
            } else {
                ChannelRef::Input(n)
            });
        }
        // -- WP CLEAN1 fixup cl1 -- a negative id is refused by the table itself
        let id = self
            .read
            .whole(e.get("channelId"), i64::from(i32::MIN), i64::from(i32::MAX))?;
        preset_file::channel_id::try_resolve(i32::try_from(id).ok()?).ok()
    }

    fn global(&mut self, inputs: usize) {
        let g = self.read.object(self.doc.get("global"));
        let n_in = usize::from(self.inputs());

        let mut whole = false;
        if let Some(list) = self.read.list(g.and_then(|g| g.get("inputPreampsDb"))) {
            whole = list.len() >= inputs;
            for (i, v) in list.iter().enumerate().take(n_in) {
                match self.read.real(Some(v)) {
                    Some(db) => {
                        if let Some(p) = self.out.state.input_preamp_db.get_mut(i) {
                            *p = db;
                        }
                        if i == 0 {
                            self.out.state.preamp_db = db;
                        }
                    }
                    None => whole = false,
                }
            }
        }
        if !whole {
            self.leave("global.inputPreampsDb");
        }
        self.fields(g, "global", &global_fields());
        if self.options & IMPORT_VOLUMES != 0 {
            self.fields(g, "global", &volume_fields());
        }

        if let Some(list) = self.read.list(g.and_then(|g| g.get("inputPairLinked"))) {
            let mut mask = 0u8;
            for (p, v) in list.iter().enumerate().take(4) {
                if self.read.switch(Some(v)) == Some(true) {
                    mask |= 1 << p;
                }
            }
            self.out.links = Some(mask);
        }
    }

    /// One placed channel entry.
    fn channel(&mut self, place: ChannelRef, e: &Map<String, Value>) {
        let wire = usize::from(place.unified(self.inputs()));
        let name = self.read.text(e.get("name")).filter(|n| !n.is_empty());
        let shown = name
            .clone()
            .or_else(|| self.out.state.channel_names.get(wire).cloned())
            .unwrap_or_else(|| format!("channel {wire}"));
        if let (Some(n), Some(slot)) = (name, self.out.state.channel_names.get_mut(wire)) {
            *slot = n;
        }

        let line = state::max_delay_ms_for(&self.out.state);
        match self.delay(e.get("delayMs"), line, &shown) {
            Some(ms) => {
                if let Some(d) = self.out.state.channel_delays.get_mut(wire) {
                    *d = ms;
                }
            }
            None => self.leave("channels.delayMs"),
        }

        match self.bank(e.get("eq"), BANDS_PER_CHANNEL, false) {
            Some(bank) => replace_bank(self.out.state.filters.get_mut(wire), &bank),
            None => self.leave("channels.eq"),
        }

        let ChannelRef::Output(o) = place else {
            return;
        };
        let o = usize::from(o);
        let s = &mut self.out.state;
        if let Some(v) = self.read.real(e.get("gainDb")) {
            set(&mut s.output_gain_db, o, v);
        } else {
            self.leave("channels.gainDb");
        }
        let s = &mut self.out.state;
        if let Some(v) = self.read.switch(e.get("muted")) {
            set(&mut s.output_muted, o, v);
        } else {
            self.leave("channels.muted");
        }
        let s = &mut self.out.state;
        if let Some(v) = self.read.switch(e.get("enabled")) {
            set(&mut s.output_enabled, o, v);
        } else {
            self.leave("channels.enabled");
        }
        match self.delay(
            e.get("outputDelayMs"),
            line,
            &format!("{shown} output delay"),
        ) {
            Some(ms) => set(&mut self.out.state.output_delay_ms, o, ms),
            None => self.leave("channels.outputDelayMs"),
        }
        match self.bank(e.get("crossover"), MAX_XOVER_BANDS, true) {
            Some(bank) => replace_bank(self.out.state.xover.get_mut(wire), &bank),
            None => self.leave("channels.crossover"),
        }
        if self.options & IMPORT_HARDWARE != 0
            && let Some(l) = self.read.object(e.get("limiter"))
        {
            self.limiter(l, o);
        }
    }

    /// A delay held to the device's line `line`; one past it is shortened
    /// and named.
    fn delay(&mut self, v: Option<&Value>, line: f32, what: &str) -> Option<f32> {
        let ms = self.read.real(v)?;
        if ms > line {
            self.out.shortened.push(format!("{what} ({ms} ms)"));
            return Some(line);
        }
        Some(ms.max(0.0))
    }

    /// A channel's `eq` or `crossover`: `None` (left alone) unless it is a
    /// list with entries. The entries replace the bank, the rest going flat.
    fn bank(
        &mut self,
        v: Option<&Value>,
        width: usize,
        crossover: bool,
    ) -> Option<Vec<FilterParams>> {
        let list = self.read.list(v).filter(|l| !l.is_empty())?;
        if list.len() > width {
            self.out.skipped.add(format!(
                "bands past the {width} a {} holds were dropped",
                if crossover {
                    "crossover bank"
                } else {
                    "channel's EQ"
                }
            ));
        }
        let taken = list.len().min(width);
        if crossover {
            self.out.xover = self.out.xover.saturating_add(taken);
        } else {
            self.out.bands = self.out.bands.saturating_add(taken);
        }
        Some(
            (0..width)
                .map(|k| match list.get(k) {
                    Some(b) => {
                        let band = self.band(b);
                        self.fit(band, crossover)
                    }
                    None => flat(),
                })
                .collect(),
        )
    }

    fn band(&mut self, v: &Value) -> FilterParams {
        let mut b = flat();
        let Some(o) = self.read.object(Some(v)) else {
            return b;
        };
        if let Some(t) = self.read.byte(o.get("type")) {
            b.filter_type = t;
        }
        if let Some(x) = self.read.real(o.get("freqHz")) {
            b.freq = x;
        }
        if let Some(x) = self.read.real(o.get("q")) {
            b.q = x;
        }
        if let Some(x) = self.read.real(o.get("gain")) {
            b.gain = x;
        }
        if let Some(x) = self.read.real(o.get("qp")) {
            b.qp = x;
        }
        if let Some(x) = self.read.switch(o.get("bypass")) {
            b.bypass = x;
        }
        b
    }

    /// A band whose type does not belong in its bank goes flat, and says so.
    fn fit(&mut self, b: FilterParams, crossover: bool) -> FilterParams {
        let t = FilterType::from_raw(b.filter_type);
        let reason = if crossover {
            (t != FilterType::Flat && !t.is_crossover()).then(|| {
                format!(
                    "{} bands in a crossover bank were set to Off",
                    t.display_name()
                )
            })
        } else if t.is_crossover() {
            Some(format!(
                "{} bands in an EQ bank were set to Off",
                t.display_name()
            ))
        } else {
            (!t.is_known()).then(|| {
                format!(
                    "{} bands (not supported by this firmware) were set to Off",
                    t.display_name()
                )
            })
        };
        match reason {
            Some(r) => {
                self.out.skipped.add(r);
                flat()
            }
            None => b,
        }
    }

    fn limiter(&mut self, l: &Map<String, Value>, o: usize) {
        for (k, which, o) in limiter_fields(o) {
            let s = &mut self.out.state;
            let done = match which {
                LimiterKey::Enabled => self
                    .read
                    .switch(l.get(k))
                    .map(|v| set(&mut s.limiter_enabled, o, v)),
                LimiterKey::Threshold => self
                    .read
                    .real(l.get(k))
                    .map(|v| set(&mut s.limiter_threshold_db, o, v)),
                LimiterKey::Release => self
                    .read
                    .real(l.get(k))
                    .map(|v| set(&mut s.limiter_release_ms, o, v)),
                LimiterKey::LinkGroup => self
                    .read
                    .byte(l.get(k))
                    .map(|v| set(&mut s.limiter_link_group, o, v)),
            };
            if done.is_none() {
                self.leave(format!("channels.limiter.{k}"));
            }
        }
    }

    fn matrix(&mut self) {
        let list = self.read.list(self.doc.get("matrix")).unwrap_or_default();
        if list.is_empty() {
            self.leave("matrix");
            return;
        }
        let (n_in, n_out) = (self.inputs(), self.outputs());
        for v in list {
            let Some(e) = self.read.object(Some(v)) else {
                continue;
            };
            let (Some(i), Some(o)) = (
                self.read.byte(e.get("input")),
                self.read.byte(e.get("output")),
            ) else {
                self.read.unreadable = true;
                continue;
            };
            if i >= n_in || o >= n_out {
                continue;
            }
            let (i, o) = (usize::from(i), usize::from(o));
            let s = &mut self.out.state;
            match self.read.switch(e.get("enabled")) {
                Some(on) => set_cell(&mut s.matrix_routing, i, o, on),
                None => self.leave("matrix.enabled"),
            }
            let s = &mut self.out.state;
            match self.read.switch(e.get("invert")) {
                Some(inv) => set_cell(&mut s.matrix_invert, i, o, inv),
                None => self.leave("matrix.invert"),
            }
            let s = &mut self.out.state;
            match self.read.real(e.get("gainDb")) {
                Some(db) => set_cell(&mut s.matrix_gain, i, o, db),
                None => self.leave("matrix.gainDb"),
            }
            self.out.crosspoints = self.out.crosspoints.saturating_add(1);
        }
    }

    fn feature_blocks(&mut self) {
        self.block("loudness", &loudness_fields());
        self.block("crossfeed", &crossfeed_fields());
        self.pair_map();
        self.block("leveller", &leveller_fields());

        // A feature block the document lacks says the source had no such
        // feature, so it is not named.
        let rp2350 = Platform::from_id(self.out.state.platform_id) == Platform::Rp2350;
        let optional: [(&str, Vec<Field>, Option<&str>); 4] = [
            ("psybass", psybass_fields(), None),
            (
                "upmix",
                upmix_fields(),
                (!rp2350).then_some("Stereo upmixer (not supported by this device)"),
            ),
            ("subharm", subharm_fields(), None),
            ("tube", tube_fields(), None),
        ];
        for (name, fields, refusal) in optional {
            if given(self.doc.get(name)).is_none() {
                continue;
            }
            match refusal {
                Some(r) => self.out.skipped.add(r),
                None => self.block(name, &fields),
            }
        }
    }

    /// `crossfeed.pairMap`, as the Console takes it: the normalized map when
    /// it is valid for this platform's pairs, otherwise the default pairing.
    fn pair_map(&mut self) {
        let cf = self.doc.get("crossfeed").and_then(Value::as_object);
        let Some(list) = self.read.list(cf.and_then(|c| c.get("pairMap"))) else {
            return;
        };
        let mut bytes = [0u8; PairMap::SIZE];
        if list.len() <= PairMap::SIZE {
            for (b, v) in bytes.iter_mut().zip(list) {
                *b = self.read.byte(Some(v)).unwrap_or(0);
            }
        }
        let map = PairMap(bytes);
        let map = if map.is_valid(usize::from(crossfeed_pairs(&self.out.state))) {
            map.normalized().0
        } else {
            [0; PairMap::SIZE]
        };
        self.out.state.crossfeed_pair_map = map.to_vec();
        self.out.pair_map = Some(map);
    }

    /// The hardware I/O (`IMPORT_HARDWARE`). Its keys depend on the source
    /// board, so an absent one is left alone without being named.
    fn io(&mut self) {
        let Some(io) = self.read.object(self.doc.get("io")) else {
            return;
        };
        let applied = self.fields(Some(io), "io", &io_fields());
        // Not named: see above.
        self.out.left_alone.retain(|k| !k.starts_with("io."));
        let s = &mut self.out.state;
        s.i2s_clock_pin_mode_present |= applied.contains(&"i2sClockPinMode");
        s.adat_input_enabled_present |= applied.contains(&"adatInputEnabled");
        s.adat_input_clock_mode_present |= applied.contains(&"adatInputClockMode");

        let pins = usize::from(self.out.state.num_pin_outputs);
        self.pins(io.get("outputPins"), pins, |s| &mut s.output_pins);
        let types = self.out.state.output_types.len();
        self.pins(io.get("outputSlotTypes"), types, |s| &mut s.output_types);
        self.pins(io.get("spdifRxPins"), 3, |s| &mut s.spdif_rx_pins);
        self.pins(io.get("i2sRxPins"), 4, |s| &mut s.i2s_rx_pins);
        if let Some(p) = self.read.byte(io.get("spdifRxPin4")) {
            set(&mut self.out.state.spdif_rx_pins, 3, p);
        }
        if let Some(mask) = self.read.byte(io.get("spdifEnabledExt")) {
            let s = &mut self.out.state;
            s.spdif_inputs_enabled = 0x01 | ((mask & 0x07) << 1);
            s.spdif_inputs_enabled_present = true;
        }
        if let Some(m) = self.read.whole(io.get("mckMultiplier"), 0, 1024) {
            match m {
                128 => self.out.state.mck_multiplier = 0,
                256 => self.out.state.mck_multiplier = 1,
                _ => self.read.unreadable = true,
            }
        }
        if let Some(hz) = self.read.whole(io.get("i2sInputRateHz"), 0, 1_000_000) {
            match hz {
                44_100 => self.out.state.i2s_input_rate = 0,
                48_000 => self.out.state.i2s_input_rate = 1,
                96_000 => self.out.state.i2s_input_rate = 2,
                _ => self.read.unreadable = true,
            }
        }
        if let Some(mute) = self.read.object(io.get("dacHwMute")) {
            if self.out.state.dac_mute_supported {
                self.dac_mute(mute);
            } else {
                self.out
                    .skipped
                    .add("External DAC hardware mute (not supported by this firmware)");
            }
        }
    }

    /// A list of pins (or slot types) onto the first `width` entries of a
    /// mirror list; an entry of the wrong type keeps its value.
    fn pins(&mut self, v: Option<&Value>, width: usize, to: fn(&mut DspState) -> &mut Vec<u8>) {
        let Some(list) = self.read.list(v) else {
            return;
        };
        for (k, p) in list.iter().enumerate().take(width) {
            if let Some(p) = self.read.byte(Some(p)) {
                set(to(&mut self.out.state), k, p);
            }
        }
    }

    fn dac_mute(&mut self, m: &Map<String, Value>) {
        let s = &mut self.out.state;
        if let Some(v) = self.read.switch(m.get("enabled")) {
            s.dac_mute_enabled = v;
        }
        if let Some(v) = self.read.switch(m.get("activeLow")) {
            s.dac_mute_active_low = v;
        }
        if let Some(v) = self.read.byte(m.get("pin")) {
            s.dac_mute_pin = v;
        }
        if let Some(v) = self.read.word(m.get("holdMs")) {
            s.dac_mute_hold_ms = v;
        }
        if let Some(v) = self.read.word(m.get("releaseMs")) {
            s.dac_mute_release_ms = v;
        }
    }

    /// A document from older firmware may hold 0 dB or off for settings that
    /// firmware never reported, if an older build wrote it.
    fn old_format(&mut self) {
        let wire = self
            .doc
            .get("meta")
            .and_then(|m| m.get("wireFormatVersion"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let ours = i64::from(dspi_proto::generated::wire::WIRE_FORMAT_VERSION);
        if wire > 0 && wire < ours {
            self.out.warnings.push(format!(
                "This file comes from older firmware (wire format V{wire}; this build speaks \
                 V{ours}). An export from that firmware by an older build can hold 0 dB or \
                 off for settings the firmware did not report, so check the levels."
            ));
        }
    }
}

/// A bank replaced by `bands`, its other slots flat; its length stays.
fn replace_bank(bank: Option<&mut Vec<FilterParams>>, bands: &[FilterParams]) {
    for (k, slot) in bank.into_iter().flatten().enumerate() {
        *slot = bands.get(k).copied().unwrap_or_else(flat);
    }
}

fn set<T>(list: &mut [T], at: usize, v: T) {
    if let Some(slot) = list.get_mut(at) {
        *slot = v;
    }
}

fn set_cell<T>(grid: &mut [Vec<T>], row: usize, col: usize, v: T) {
    if let Some(r) = grid.get_mut(row) {
        set(r, col, v);
    }
}

/// The report of an import, worded as the Console words it, and whether it is
/// clean: nothing missing, skipped, left alone, shortened or unreadable.
fn config_report(b: &Built) -> (Vec<String>, bool) {
    let mut lines = vec![format!(
        "Applied {} channels, {} EQ bands, {} crossover bands, {} crosspoints.",
        b.channels, b.bands, b.xover, b.crosspoints
    )];
    lines.extend(b.missing.line());
    lines.extend(b.skipped.reasons.iter().map(|r| format!("Skipped: {r}")));
    if !b.left_alone.is_empty() {
        lines.push(format!(
            "Left as they are, because the file holds no value for them: {}",
            b.left_alone.join(", ")
        ));
    }
    if !b.shortened.is_empty() {
        lines.push(format!(
            "Delays shortened to the {} ms this device ({}) holds: {}.",
            state::max_delay_ms_for(&b.state),
            Platform::from_id(b.state.platform_id).name(),
            b.shortened.join(", ")
        ));
    }
    if b.unreadable {
        lines.push(
            "Some values in the file could not be read; those settings were left as they are."
                .into(),
        );
    }
    lines.extend(b.warnings.iter().map(|w| format!("Warning: {w}")));
    lines.push(NOT_STORED.into());
    let clean = b.missing.is_empty()
        && b.skipped.is_empty()
        && b.left_alone.is_empty()
        && b.shortened.is_empty()
        && !b.unreadable
        && b.warnings.is_empty();
    (lines, clean)
}

/// `{"ok": false, "error": ...}`.
fn refusal(error: &str) -> String {
    json!({ "ok": false, "error": error }).to_string()
}

/// Why nothing (more) could be written, for the user.
fn write_error(e: &DspiError) -> String {
    match e {
        DspiError::Disconnected => "No device connected.".into(),
        DspiError::ReadOnly => {
            "This device's firmware is read-only in this version; nothing was written.".into()
        }
        DspiError::Locked => {
            "Another console is applying changes to this device. Try again in a moment.".into()
        }
        DspiError::Denied => "This console's role on the hub does not allow changes.".into(),
        other => format!("The device could not be written: {other}."),
    }
}

/// The mirror, if it holds a device's state to export, and the pair map the
/// extension channel reports. `Disconnected` with no device, `Unsupported`
/// for firmware whose image this build cannot map, `Transfer` before any
/// image was read.
fn exportable(inner: &mut Inner) -> Result<(DspState, Option<[u8; 4]>)> {
    let conn = inner.conn()?;
    let wire = conn.wire;
    let ext = conn.ext.pair_map;
    let s = &inner.state;
    if wire.is_none() {
        return Err(
            if matches!(s.compat, COMPAT_FIRMWARE_TOO_OLD | COMPAT_FIRMWARE_TOO_NEW) {
                DspiError::Unsupported
            } else {
                DspiError::Transfer {
                    msg: "no settings have been read from the device yet".into(),
                }
            },
        );
    }
    // This build's own image carries the map and is kept current; an older
    // image cannot hold it, so the connection's copy is what the device said.
    let map = ext.map(
        |m| match (wire, <[u8; 4]>::try_from(s.crossfeed_pair_map.as_slice())) {
            (Some(WireRead::Exact), Ok(own)) => own,
            _ => m.0,
        },
    );
    Ok((s.clone(), map))
}

/// Check a `.dspipreset` document and describe it, as JSON
/// `{ok, error, name, platform, firmware, savedUtc}`. `ok` is false, with the
/// reason in `error`, for text that is not a document this build reads: not
/// JSON, no schema version or a newer one, no channels, or over
/// [`MAX_FILE_BYTES`]. Needs no device.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn config_inspect(text: String) -> String {
    let doc = match parse_config(&text) {
        Ok(d) => d,
        Err(e) => return refusal(&e),
    };
    let meta = doc.get("meta");
    let field = |k: &str| {
        meta.and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    json!({
        "ok": true,
        "error": "",
        "name": field("name"),
        "platform": field("platform"),
        "firmware": field("firmwareVersion"),
        "savedUtc": field("savedUtc"),
    })
    .to_string()
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The device's configuration as a `.dspipreset` document (JSON, schema
    /// 1), from the mirror: no transfer. `name` and `app_version` go into
    /// `meta`; `links` is the app's input-pair link mask (bit p: pair p),
    /// written as `global.inputPairLinked`.
    ///
    /// The document holds what the device's wire format carries: a key or
    /// block that firmware lacks is left out, `crossfeed.pairMap` is written
    /// only when the device reports a map over the extension channel, and
    /// the upmixer only on an RP2350. A read-only device (another wire
    /// format) exports what its image showed.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::Transfer`] before its settings were read;
    /// [`DspiError::Unsupported`] for firmware whose image this build cannot
    /// map.
    pub fn config_export(&self, name: String, app_version: String, links: u8) -> Result<String> {
        self.ensure_probed();
        let (state, map) = exportable(&mut self.lock())?;
        Ok(export_config(&state, &name, &app_version, links, map))
    }

    /// Apply a `.dspipreset` document to the device as one bulk write, under
    /// the device lock. `options`: [`IMPORT_VOLUMES`] also applies the master
    /// and user volume, [`IMPORT_HARDWARE`] the hardware I/O and the
    /// limiters. The crossfeed pair map goes over the extension channel when
    /// the device has one.
    ///
    /// Every setting the document holds no readable value for keeps the
    /// device's value: a missing key or block, a value of the wrong type, an
    /// `eq` or `crossover` with no entries. A bank with entries replaces the
    /// channel's, the rest going flat. Delays past the device's delay line
    /// are shortened to it. Channels the device lacks are named, not moved.
    ///
    /// Returns JSON. On success `{ok: true, error: "", lines, clean, links,
    /// hardwareWritten, leftAlone}`: `lines` is the report, `clean` false
    /// when anything was missing, skipped, left alone or shortened,
    /// `leftAlone` the keys left alone, `links` the input-pair link mask to
    /// restore (null when the document has none), and `hardwareWritten`
    /// whether I/O was changed while the device stores its output
    /// configuration apart from presets (save that to keep it). Otherwise
    /// `{ok: false, error}`: the text is not a document this build reads, or
    /// the device could not be written (disconnected, read-only firmware,
    /// another console's lock, a failed transfer); the mirror then stays as
    /// it was.
    pub fn config_import(&self, text: String, options: u32) -> String {
        let doc = match parse_config(&text) {
            Ok(d) => d,
            Err(e) => return refusal(&e),
        };
        self.ensure_probed();
        match self.import_document(&doc, options) {
            Ok(reply) => reply.to_string(),
            Err(e) => refusal(&write_error(&e)),
        }
    }
}

impl DspiCore {
    fn import_document(&self, doc: &Value, options: u32) -> Result<Value> {
        let lock = self.acquire_device_lock(LOCK_REASON)?;
        let result = (|| {
            let (mut built, before, separate) = {
                let mut guard = self.lock();
                let inner = &mut *guard;
                if !lock.is_current(inner) {
                    return Err(DspiError::Disconnected);
                }
                self.write_document(inner, doc, options)?
            };
            let hardware = options & IMPORT_HARDWARE != 0;
            let written = hardware && separate && io_changed(&before, &built.state);
            if written {
                self.push_output_config(&before, &built.state, &mut built.skipped);
            }
            let (lines, clean) = config_report(&built);
            // -- WP P0M --
            let names = self.lock().conn.as_ref().map(|c| c.source_names.to_vec());
            let changed =
                crate::presets::changes(&before, &built.state, names.as_deref().unwrap_or(&[]));
            self.history.applied(
                dspi_session::events::EventKind::Import,
                None,
                "Imported a configuration",
                changed,
            );
            Ok(json!({
                "ok": true,
                "error": "",
                "lines": lines,
                "clean": clean,
                "links": built.links,
                "hardwareWritten": written,
                "leftAlone": built.left_alone,
            }))
        })();
        self.release_device_lock(lock);
        result
    }

    /// Build the document onto the mirror and send it. Returns what was
    /// built, the state before, and whether the device keeps its output
    /// configuration apart from presets (`OUTPUT_CONFIG_MODE_INDEPENDENT`).
    fn write_document(
        &self,
        inner: &mut Inner,
        doc: &Value,
        options: u32,
    ) -> Result<(Built, DspState, bool)> {
        let before = inner.state.clone();
        let mut built = build_config(&before, doc, options);
        let has_map = inner.conn()?.ext.pair_map.is_some();
        if !has_map && let Some(map) = built.pair_map {
            built.state.crossfeed_pair_map = before.crossfeed_pair_map.clone();
            if map != [0; PairMap::SIZE] {
                built
                    .skipped
                    .add("Crossfeed output pairs (not supported by this firmware)");
            }
        }
        inner.state = built.state.clone();
        if let Err(e) = apply_all_params_locked(inner) {
            inner.state = before;
            return Err(e);
        }
        if has_map
            && let Some(map) = built.pair_map
            && let Err(e) = set_pair_map_locked(inner, &map)
        {
            built
                .skipped
                .add(format!("Crossfeed output pairs could not be written ({e})"));
        }
        built.state = inner.state.clone();
        Ok((built, before, inner.state.output_config_mode == 0))
    }

    /// With the output configuration kept apart from presets, the firmware
    /// takes no pins and no limiter from a bulk write: send them one by one.
    /// A refusal is reported, and the mirror read again.
    fn push_output_config(&self, before: &DspState, after: &DspState, skipped: &mut Skips) {
        let mut failed = false;
        let mut attempt = |what: String, r: Result<()>| {
            if let Err(e) = r {
                skipped.add(format!("{what} rejected by the device ({e})"));
                failed = true;
            }
        };
        let changed = |a: &[u8], b: &[u8], k: usize| a.get(k) != b.get(k);
        for slot in 0..after.num_pin_outputs {
            let k = usize::from(slot);
            if let Some(&pin) = after.output_pins.get(k)
                && changed(&before.output_pins, &after.output_pins, k)
            {
                attempt(
                    format!("Output {} GPIO", k.saturating_add(1)),
                    self.set_output_pin(slot, pin),
                );
            }
        }
        for (k, index) in (0..4u8).enumerate() {
            if let Some(&pin) = after.spdif_rx_pins.get(k)
                && changed(&before.spdif_rx_pins, &after.spdif_rx_pins, k)
            {
                attempt(
                    format!("S/PDIF input {} pin", k.saturating_add(1)),
                    self.set_spdif_rx_pin(index, pin),
                );
            }
        }
        for index in 1..4u8 {
            let bit = 1u8 << index;
            let want = after.spdif_inputs_enabled & bit != 0;
            if want != (before.spdif_inputs_enabled & bit != 0) {
                attempt(
                    format!("S/PDIF input {}", index.saturating_add(1)),
                    self.set_spdif_input_enabled(index, want),
                );
            }
        }
        if after.i2s_input_channels != before.i2s_input_channels {
            attempt(
                "I2S input channels".into(),
                self.set_i2s_input_channels(after.i2s_input_channels),
            );
        }
        for (k, pair) in (0..4u8).enumerate() {
            if let Some(&pin) = after.i2s_rx_pins.get(k)
                && changed(&before.i2s_rx_pins, &after.i2s_rx_pins, k)
            {
                attempt(
                    format!("I2S serial data {} pin", k.saturating_add(1)),
                    self.set_i2s_rx_pin(pair, pin),
                );
            }
        }
        if after.adat_input_pin != before.adat_input_pin && after.adat_input_pin != 0xFF {
            attempt(
                "ADAT input pin".into(),
                self.set_adat_input_pin(after.adat_input_pin),
            );
        }
        if after.adat_input_enabled != before.adat_input_enabled {
            attempt(
                "ADAT input enable".into(),
                self.set_adat_input_enabled(after.adat_input_enabled),
            );
        }
        for out in 0..after.num_output_channels {
            let o = usize::from(out);
            let real = |v: &[f32]| v.get(o).copied();
            let params: [(u16, Option<f32>, Option<f32>); 4] = [
                (
                    limiter::LIMITER_PARAM_ENABLED,
                    before
                        .limiter_enabled
                        .get(o)
                        .map(|&e| f32::from(u8::from(e))),
                    after
                        .limiter_enabled
                        .get(o)
                        .map(|&e| f32::from(u8::from(e))),
                ),
                (
                    limiter::LIMITER_PARAM_THRESHOLD_DB,
                    real(&before.limiter_threshold_db),
                    real(&after.limiter_threshold_db),
                ),
                (
                    limiter::LIMITER_PARAM_RELEASE_MS,
                    real(&before.limiter_release_ms),
                    real(&after.limiter_release_ms),
                ),
                (
                    limiter::LIMITER_PARAM_LINK_GROUP,
                    before.limiter_link_group.get(o).map(|&g| f32::from(g)),
                    after.limiter_link_group.get(o).map(|&g| f32::from(g)),
                ),
            ];
            for (param, had, want) in params {
                if let Some(want) = want
                    && had != Some(want)
                {
                    attempt(
                        format!("Output {} limiter", o.saturating_add(1)),
                        self.set_limiter_param(out, param as u8, want),
                    );
                }
            }
        }
        if failed {
            let _ = refresh_params_locked(&mut self.lock());
        }
    }
}

/// Whether anything of the output configuration (pins, slot types, clocks,
/// inputs, the DAC mute, the limiters) differs.
fn io_changed(a: &DspState, b: &DspState) -> bool {
    a.output_pins != b.output_pins
        || a.output_types != b.output_types
        || a.i2s_bck_pin != b.i2s_bck_pin
        || a.i2s_bck_pin_slave != b.i2s_bck_pin_slave
        || a.i2s_clock_mode != b.i2s_clock_mode
        || a.i2s_clock_pin_mode != b.i2s_clock_pin_mode
        || a.mck_enabled != b.mck_enabled
        || a.mck_pin != b.mck_pin
        || a.mck_multiplier != b.mck_multiplier
        || a.spdif_rx_pins != b.spdif_rx_pins
        || a.spdif_inputs_enabled != b.spdif_inputs_enabled
        || a.i2s_rx_pins != b.i2s_rx_pins
        || a.i2s_input_channels != b.i2s_input_channels
        || a.i2s_input_rate != b.i2s_input_rate
        || a.adat_out_enabled != b.adat_out_enabled
        || a.adat_out_pin != b.adat_out_pin
        || a.adat_input_enabled != b.adat_input_enabled
        || a.adat_input_pin != b.adat_input_pin
        || a.adat_input_clock_mode != b.adat_input_clock_mode
        || a.dac_mute_enabled != b.dac_mute_enabled
        || a.dac_mute_active_low != b.dac_mute_active_low
        || a.dac_mute_pin != b.dac_mute_pin
        || a.dac_mute_hold_ms != b.dac_mute_hold_ms
        || a.dac_mute_release_ms != b.dac_mute_release_ms
        || a.limiter_enabled != b.limiter_enabled
        || a.limiter_threshold_db != b.limiter_threshold_db
        || a.limiter_release_ms != b.limiter_release_ms
        || a.limiter_link_group != b.limiter_link_group
}

// ═════════════════════════════════════════════════════════════════════
// Filter files: lines
// ═════════════════════════════════════════════════════════════════════

/// The bank a band line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bank {
    Peq,
    Xover,
}

/// The two-letter (or longer) code a band's type is written with, `None`
/// for flat and for a type this build does not know.
fn type_code(t: FilterType) -> Option<String> {
    let code = match t {
        FilterType::Flat => return None,
        FilterType::Peaking => "PK",
        FilterType::LowShelf => "LS",
        FilterType::HighShelf => "HS",
        FilterType::LowPass => "LP",
        FilterType::HighPass => "HP",
        FilterType::Notch => "NT",
        FilterType::AllPass => "AP",
        FilterType::AllPass1 => "AP1",
        FilterType::LowShelf1 => "LS1",
        FilterType::HighShelf1 => "HS1",
        FilterType::LinkwitzTransform => "LT",
        FilterType::LowPass1 => "LP1",
        FilterType::HighPass1 => "HP1",
        FilterType::Unknown(raw) => {
            return xover::meta(raw).map(|m| m.label().replace(' ', ""));
        }
    };
    Some(code.to_string())
}

/// A code back to its type, with the aliases other tools write: REW's PEQ,
/// LSC/HSC (the second-order shelves), LPQ/HPQ, and NO for a notch.
fn code_type(code: &str) -> Option<FilterType> {
    let up = code.to_ascii_uppercase();
    let t = match up.as_str() {
        "PK" | "PEQ" => FilterType::Peaking,
        "LS" | "LSC" => FilterType::LowShelf,
        "HS" | "HSC" => FilterType::HighShelf,
        "LP" | "LPQ" => FilterType::LowPass,
        "HP" | "HPQ" => FilterType::HighPass,
        "NT" | "NO" => FilterType::Notch,
        "AP" => FilterType::AllPass,
        "AP1" => FilterType::AllPass1,
        "LS1" => FilterType::LowShelf1,
        "HS1" => FilterType::HighShelf1,
        "LP1" => FilterType::LowPass1,
        "HP1" => FilterType::HighPass1,
        "LT" => FilterType::LinkwitzTransform,
        _ => {
            return (xover::XOVER_FIRST..=xover::XOVER_LAST)
                .map(FilterType::from_raw)
                .find(|t| type_code(*t).as_deref() == Some(up.as_str()));
        }
    };
    Some(t)
}

/// The Windows Console's crossover spelling: family, `LP`/`HP`, and
/// `Slope NN dB/oct`. An order the family does not have matches nothing.
fn spelled_crossover(family: &str, shape: Option<&str>, slope: Option<f32>) -> Option<FilterType> {
    let family = match family.to_ascii_uppercase().as_str() {
        "LR" => xover::Family::LinkwitzRiley,
        "BW" => xover::Family::Butterworth,
        "BES" | "BESSEL" => xover::Family::Bessel,
        _ => return None,
    };
    let high_pass = match shape?.to_ascii_uppercase().as_str() {
        "HP" => true,
        "LP" => false,
        _ => return None,
    };
    let slope = slope.filter(|s| *s >= 6.0)?;
    let order = (slope / 6.0).floor() as u8;
    (xover::XOVER_FIRST..=xover::XOVER_LAST)
        .find(|raw| {
            xover::meta(*raw)
                .is_some_and(|m| m.family == family && m.order == order && m.high_pass == high_pass)
        })
        .map(FilterType::from_raw)
}

/// A number in a filter file: a glued unit (`100Hz`) is dropped, and either
/// separator is a decimal point. With both present the last one is, the
/// other grouping thousands (the Windows Console's rule), so `1.234,5` and
/// `0,707` read right.
fn number(token: &str) -> Option<f32> {
    let digits = token.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != ',');
    if digits.is_empty() {
        return None;
    }
    let text = match (digits.rfind('.'), digits.rfind(',')) {
        (Some(dot), Some(comma)) if dot > comma => digits.replace(',', ""),
        (Some(_), Some(_)) => digits.replace('.', "").replace(',', "."),
        _ => digits.replace(',', "."),
    };
    text.parse::<f32>().ok().filter(|x| x.is_finite())
}

/// `Preamp -6.5 dB` or REW's `Preamp: -6.5 dB`.
fn preamp_line(line: &str) -> Option<f32> {
    let line = line.trim();
    if !line.to_ascii_lowercase().starts_with("preamp") {
        return None;
    }
    line.get(6..)?
        .split(|c: char| c.is_whitespace() || c == ':')
        .filter(|t| !t.is_empty())
        .find_map(number)
}

/// `# Format: N`.
fn format_line(line: &str) -> Option<u32> {
    let line = line.trim();
    if !line.to_ascii_lowercase().starts_with("# format") {
        return None;
    }
    line.split(|c: char| c.is_whitespace() || c == ':')
        .find_map(|t| t.parse::<u32>().ok())
}

/// A band line: its bank, and the band (`None` for an `OFF` line or a type
/// this build cannot read). `None` for any other line.
fn band_line(line: &str) -> Option<(Bank, Option<FilterParams>)> {
    let line = line.trim();
    let lower = line.to_ascii_lowercase();
    let (word, bank) = [
        ("crossover", Bank::Xover),
        ("filter", Bank::Peq),
        ("xover", Bank::Xover),
    ]
    .into_iter()
    .find(|(w, _)| lower.starts_with(w))?;
    // "<keyword> <n>:" — the number keeps prose such as "Filters: 5" out.
    let colon = line.find(':')?;
    let index = line.get(word.len()..colon)?.trim();
    if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let tokens: Vec<&str> = line
        .get(colon.saturating_add(1)..)?
        .split_whitespace()
        .collect();
    let (Some(state), Some(code)) = (tokens.first(), tokens.get(1)) else {
        return Some((bank, None));
    };
    if !state.eq_ignore_ascii_case("on") {
        return Some((bank, None));
    }
    let rest = tokens.get(2..).unwrap_or_default();
    let values = labelled(rest);
    let value = |label: &str| {
        values
            .iter()
            .rev()
            .find(|(l, _)| l == label)
            .map(|(_, v)| *v)
    };
    let Some(t) =
        code_type(code).or_else(|| spelled_crossover(code, rest.first().copied(), value("SLOPE")))
    else {
        return Some((bank, None));
    };
    let mut b = flat();
    b.filter_type = t.to_raw();
    if let Some(v) = value("FC") {
        b.freq = v;
    }
    if let Some(v) = value("GAIN") {
        b.gain = v;
    }
    if let Some(v) = value("Q") {
        b.q = v;
    }
    if let Some(v) = value("QP") {
        b.qp = v;
    }
    if t.is_linkwitz()
        && let Some(fp) = value("FP")
    {
        b.gain = fp;
    }
    b.bypass = line.to_ascii_uppercase().contains("[BYPASSED]")
        || rest.iter().any(|t| t.eq_ignore_ascii_case("BYP"));
    Some((bank, Some(b)))
}

/// Each label token with the number that follows it; units and other words
/// pair with nothing.
fn labelled(tokens: &[&str]) -> Vec<(String, f32)> {
    tokens
        .windows(2)
        .filter_map(|w| match (w.first(), w.get(1)) {
            (Some(l), Some(v)) if number(l).is_none() => {
                number(v).map(|n| (l.to_ascii_uppercase(), n))
            }
            _ => None,
        })
        .collect()
}

/// One band as a line of the Console's format.
fn format_band(bank: Bank, index: usize, b: &FilterParams) -> String {
    let word = match bank {
        Bank::Peq => "Filter",
        Bank::Xover => "Xover",
    };
    let t = FilterType::from_raw(b.filter_type);
    let Some(code) = type_code(t) else {
        return format!("{word:<6} {index:>2}: OFF\n");
    };
    let mut line = format!("{word:<6} {index:>2}: ON  {code:<8}Fc {:7.1} Hz", b.freq);
    if t.is_linkwitz() {
        line.push_str(&format!(
            "  Q {:6.3}  Fp {:7.1} Hz  Qp {:6.3}",
            b.q, b.gain, b.qp
        ));
    } else {
        if t.uses_gain() {
            line.push_str(&format!("  Gain {:+7.3} dB", b.gain));
        }
        if t.uses_q() {
            line.push_str(&format!("  Q {:6.3}", b.q));
        }
    }
    if b.bypass {
        line.push_str("  [Bypassed]");
    }
    line.push('\n');
    line
}

/// The mirror's bands as a filter file: the first `inputs` inputs with their
/// preamp and EQ, then every output with its EQ, crossover and enable state.
fn filter_text(s: &DspState, inputs: u8, exported: u64) -> String {
    let mut out = format!(
        "# DSPi Console Filter Settings\n# Exported: {exported} (Unix time)\n# Format: {FILTER_FORMAT}\n\n"
    );
    let name = |w: usize| s.channel_names.get(w).cloned().unwrap_or_default();
    let n_in = s.num_input_channels;
    for i in 0..inputs.min(n_in) {
        let w = usize::from(i);
        out.push_str(&format!("[Input {i}: {}]\n", name(w)));
        let preamp = s.input_preamp_db.get(w).copied().unwrap_or(0.0);
        out.push_str(&format!("Preamp {preamp:+.1} dB\n"));
        write_bank(&mut out, Bank::Peq, s.filters.get(w), BANDS_PER_CHANNEL);
        out.push('\n');
    }
    for o in 0..s.num_output_channels {
        let w = usize::from(n_in).saturating_add(usize::from(o));
        let on = s
            .output_enabled
            .get(usize::from(o))
            .copied()
            .unwrap_or(false);
        out.push_str(&format!(
            "[Output {o}: {} ({})]\n",
            name(w),
            if on { "Enabled" } else { "Disabled" }
        ));
        write_bank(&mut out, Bank::Peq, s.filters.get(w), BANDS_PER_CHANNEL);
        write_bank(&mut out, Bank::Xover, s.xover.get(w), MAX_XOVER_BANDS);
        out.push('\n');
    }
    out
}

fn write_bank(out: &mut String, bank: Bank, bands: Option<&Vec<FilterParams>>, width: usize) {
    for (k, b) in bands.into_iter().flatten().take(width).enumerate() {
        out.push_str(&format_band(bank, k.saturating_add(1), b));
    }
}

// ═════════════════════════════════════════════════════════════════════
// Filter files: reading
// ═════════════════════════════════════════════════════════════════════

/// One channel's section of a Console file. A bank is `None` when the
/// section has no line for it, which leaves that bank alone; an `OFF` line
/// is a flat band in its slot.
#[derive(Debug, Default)]
struct Section {
    preamp: Option<f32>,
    peq: Option<Vec<Option<FilterParams>>>,
    xover: Option<Vec<Option<FilterParams>>>,
    enabled: Option<bool>,
}

impl Section {
    /// Bands that do something.
    fn active(&self, output: bool) -> usize {
        let count = |b: &Option<Vec<Option<FilterParams>>>| {
            b.iter()
                .flatten()
                .flatten()
                .filter(|p| p.filter_type != 0)
                .count()
        };
        count(&self.peq).saturating_add(if output { count(&self.xover) } else { 0 })
    }
}

/// A Console file: its format stamp (1 without one) and its sections by
/// the channel they name on this device.
struct NativeFile {
    format: u32,
    sections: BTreeMap<ChannelRef, Section>,
}

/// A REW or AutoEQ list: its enabled filters in order, and its preamp.
struct RewFile {
    filters: Vec<Option<FilterParams>>,
    preamp: Option<f32>,
}

fn without_bom(text: &str) -> &str {
    text.trim_start_matches('\u{feff}')
}

/// The Console (macOS, Windows, this one) starts its files with this line;
/// anything else is read as a REW or AutoEQ list.
fn is_native(text: &str) -> bool {
    text.trim_start().starts_with("# DSPi Console")
}

/// The channel a section header names on this device, and the enable state
/// an output header carries. The Console's own headers are keyed by index
/// (`Input 2: name`, `Output 0: name (Enabled)`); the Windows Console's by
/// default name (`Master L`, `Input 3` counting from 1, `SPDIF 2 R`, `PDM`);
/// older files by `USB L`, `Out L`, `Sub`.
fn header_place(inner: &str, s: &DspState) -> Option<(ChannelRef, Option<bool>)> {
    let (n_in, n_out) = (s.num_input_channels, s.num_output_channels);
    let pdm = n_out.saturating_sub(1);
    let exists = |r: ChannelRef| match r {
        ChannelRef::Input(i) => i < n_in,
        ChannelRef::Output(o) => o < n_out,
    };
    let own = inner.split_once(':').and_then(|(head, _)| {
        let mut words = head.split_whitespace();
        let (kind, n, extra) = (words.next()?, words.next()?, words.next());
        if extra.is_some() || !n.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let n = n.parse::<u8>().ok()?;
        match kind {
            "Input" => Some((ChannelRef::Input(n), None)),
            "Output" => {
                let tail = inner.trim_end();
                let enabled = if tail.ends_with("(Enabled)") {
                    Some(true)
                } else if tail.ends_with("(Disabled)") {
                    Some(false)
                } else {
                    None
                };
                Some((ChannelRef::Output(n), enabled))
            }
            _ => None,
        }
    });
    if let Some((r, enabled)) = own {
        return exists(r).then_some((r, enabled));
    }
    let named = match inner.trim() {
        "USB L" => Some(ChannelRef::Input(0)),
        "USB R" => Some(ChannelRef::Input(1)),
        "Out L" => Some(ChannelRef::Output(0)),
        "Out R" => Some(ChannelRef::Output(1)),
        "Sub" => Some(ChannelRef::Output(pdm)),
        _ => windows_place(inner, pdm),
    };
    named.filter(|r| exists(*r)).map(|r| (r, None))
}

/// The Windows Console's default channel names.
fn windows_place(header: &str, pdm: u8) -> Option<ChannelRef> {
    let name = header.trim().to_ascii_uppercase();
    match name.as_str() {
        "MASTER L" => return Some(ChannelRef::Input(0)),
        "MASTER R" => return Some(ChannelRef::Input(1)),
        "PDM" => return Some(ChannelRef::Output(pdm)),
        _ => {}
    }
    if let Some(n) = name.strip_prefix("INPUT ") {
        // Counted from 1; inputs 1 and 2 are always called Master L and R.
        let n = n
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (3..=8).contains(n))?;
        return Some(ChannelRef::Input(n.saturating_sub(1)));
    }
    let rest = name.strip_prefix("SPDIF ")?;
    let mut words = rest.split_whitespace();
    let (pair, side, extra) = (words.next()?, words.next()?, words.next());
    if extra.is_some() {
        return None;
    }
    let left = pair
        .parse::<u8>()
        .ok()
        .filter(|p| *p >= 1)?
        .saturating_sub(1)
        .checked_mul(2)?;
    match side {
        "L" => Some(ChannelRef::Output(left)),
        "R" => left.checked_add(1).map(ChannelRef::Output),
        _ => None,
    }
}

fn read_native(s: &DspState, text: &str) -> NativeFile {
    let mut file = NativeFile {
        format: 1,
        sections: BTreeMap::new(),
    };
    let mut current: Option<ChannelRef> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(v) = format_line(line) {
            file.format = v;
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            // An unknown header ends the section before it, so its lines
            // cannot land on the wrong channel.
            current = None;
            let inner = line.get(1..line.len().saturating_sub(1)).unwrap_or("");
            if let Some((r, enabled)) = header_place(inner, s) {
                file.sections.insert(
                    r,
                    Section {
                        enabled,
                        ..Section::default()
                    },
                );
                current = Some(r);
            }
            continue;
        }
        let Some(sec) = current.and_then(|r| file.sections.get_mut(&r)) else {
            continue;
        };
        if let Some(db) = preamp_line(line) {
            sec.preamp = Some(db);
        } else if let Some((bank, band)) = band_line(line) {
            let list = match bank {
                Bank::Peq => &mut sec.peq,
                Bank::Xover => &mut sec.xover,
            };
            list.get_or_insert_with(Vec::new).push(band);
        }
    }
    file
}

fn read_rew(text: &str) -> RewFile {
    let mut file = RewFile {
        filters: Vec::new(),
        preamp: None,
    };
    for line in text.lines() {
        if let Some(db) = preamp_line(line) {
            file.preamp = Some(db);
        } else if let Some((_, Some(b))) = band_line(line) {
            // REW numbers its own slots; the enabled ones are packed.
            file.filters.push(Some(b));
        }
    }
    file
}

/// What a filter import writes to one channel; `None` leaves that part alone.
#[derive(Debug, Clone, PartialEq)]
struct ChannelWrite {
    wire: u8,
    filters: Option<Vec<FilterParams>>,
    xover: Option<Vec<FilterParams>>,
    preamp: Option<f32>,
    /// `(output, enabled)`.
    enable: Option<(u8, bool)>,
}

impl ChannelWrite {
    fn new(wire: u8) -> Self {
        Self {
            wire,
            filters: None,
            xover: None,
            preamp: None,
            enable: None,
        }
    }

    fn writes_nothing(&self) -> bool {
        self.filters.is_none()
            && self.xover.is_none()
            && self.preamp.is_none()
            && self.enable.is_none()
    }
}

/// A filter file placed on the chosen channels: the writes, in channel order,
/// and the notes for the report.
#[derive(Debug, Default)]
struct FilterPlan {
    writes: Vec<ChannelWrite>,
    notes: Vec<String>,
}

impl FilterPlan {
    fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }

    /// `lines` onto a bank of `width`: in their slots, the rest flat. A type
    /// that does not belong (a crossover in an EQ bank, an EQ type in a
    /// crossover bank, a Linkwitz Transform on an input) is set to Off.
    fn fill(
        &mut self,
        lines: &[Option<FilterParams>],
        width: usize,
        bank: Bank,
        input: bool,
    ) -> Vec<FilterParams> {
        if lines.len() > width {
            self.note(format!(
                "{} filter lines past the {width} bands of a {} were not applied.",
                lines.len().saturating_sub(width),
                if bank == Bank::Peq {
                    "channel"
                } else {
                    "crossover"
                }
            ));
        }
        (0..width)
            .map(|k| match lines.get(k).copied().flatten() {
                Some(b) => self.allowed(b, bank, input),
                None => flat(),
            })
            .collect()
    }

    fn allowed(&mut self, b: FilterParams, bank: Bank, input: bool) -> FilterParams {
        let t = FilterType::from_raw(b.filter_type);
        let fits = match bank {
            Bank::Peq => t.is_known() && !t.is_crossover(),
            Bank::Xover => t.is_crossover(),
        };
        if !fits {
            self.note(match bank {
                Bank::Peq => "Crossover types on Filter lines were set to Off.",
                Bank::Xover => "EQ types on Xover lines were set to Off.",
            });
            return flat();
        }
        if input && t.is_linkwitz() {
            self.note("Inputs have no Linkwitz Transform; those bands were set to Off.");
            return flat();
        }
        b
    }

    fn push(&mut self, w: ChannelWrite) {
        if !w.writes_nothing() {
            self.writes.push(w);
        }
    }
}

/// The note for a preamp a file holds for an output.
const PREAMP_ON_OUTPUT: &str =
    "The file's preamp was not applied to outputs, which have no input trim.";

/// Where wire channel `wire` is on `s`.
fn wire_place(s: &DspState, wire: u8) -> Option<ChannelRef> {
    if wire < s.num_input_channels {
        Some(ChannelRef::Input(wire))
    } else if wire < s.num_channels {
        Some(ChannelRef::Output(
            wire.saturating_sub(s.num_input_channels),
        ))
    } else {
        None
    }
}

/// Plan a filter import of `text` onto the wire channels `wires` of `s`.
/// Channels the file holds nothing for, and wires the device lacks, are
/// skipped.
fn plan_filters(s: &DspState, text: &str, wires: &[u8]) -> FilterPlan {
    let text = without_bom(text);
    let mut plan = FilterPlan::default();
    let mut seen = BTreeSet::new();
    let targets: Vec<(u8, ChannelRef)> = wires
        .iter()
        .filter(|w| seen.insert(**w))
        .filter_map(|&w| wire_place(s, w).map(|r| (w, r)))
        .collect();

    if is_native(text) {
        let file = read_native(s, text);
        if file.format > FILTER_FORMAT {
            plan.note(
                "This file was written by a newer version of DSPi Console; some settings may \
                 have been ignored.",
            );
        }
        for (wire, place) in targets {
            let Some(sec) = file.sections.get(&place) else {
                continue;
            };
            let input = matches!(place, ChannelRef::Input(_));
            let mut w = ChannelWrite::new(wire);
            match &sec.peq {
                Some(lines) => {
                    w.filters = Some(plan.fill(lines, BANDS_PER_CHANNEL, Bank::Peq, input))
                }
                None => plan.note(NO_EQ_BANK),
            }
            match place {
                ChannelRef::Input(_) => {
                    w.preamp = sec.preamp;
                    if sec.xover.is_some() {
                        plan.note("Inputs have no crossover; their Xover lines were not applied.");
                    }
                }
                ChannelRef::Output(o) => {
                    match &sec.xover {
                        Some(lines) => {
                            w.xover = Some(plan.fill(lines, MAX_XOVER_BANDS, Bank::Xover, false));
                        }
                        None => plan.note(NO_XOVER_BANK),
                    }
                    w.enable = sec.enabled.map(|e| (o, e));
                    if sec.preamp.is_some() {
                        plan.note(PREAMP_ON_OUTPUT);
                    }
                }
            }
            plan.push(w);
        }
    } else {
        let file = read_rew(text);
        for (wire, place) in targets {
            let input = matches!(place, ChannelRef::Input(_));
            let mut w = ChannelWrite::new(wire);
            if file.filters.is_empty() {
                plan.note(NO_EQ_BANK);
            } else {
                w.filters = Some(plan.fill(&file.filters, BANDS_PER_CHANNEL, Bank::Peq, input));
            }
            if input {
                w.preamp = file.preamp;
            } else if file.preamp.is_some() {
                plan.note(PREAMP_ON_OUTPUT);
            }
            plan.push(w);
        }
    }
    plan
}

/// A channel picker entry.
fn choice(s: &DspState, wire: u8, bands: usize, checked: bool) -> Value {
    let label = s
        .channel_names
        .get(usize::from(wire))
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("Ch {wire}"));
    json!({ "wire": wire, "label": label, "bands": bands, "checked": checked })
}

/// [`DspiCore::filters_inspect`] on a state.
fn inspect_filters(s: &DspState, text: &str, inputs: u8) -> Value {
    if text.len() > MAX_FILE_BYTES {
        return json!({ "kind": "error", "channels": [], "error": TOO_LARGE });
    }
    let text = without_bom(text);
    let n_in = s.num_input_channels;
    if is_native(text) {
        let file = read_native(s, text);
        let channels: Vec<Value> = file
            .sections
            .iter()
            .map(|(r, sec)| {
                let output = matches!(r, ChannelRef::Output(_));
                choice(s, r.unified(n_in), sec.active(output), true)
            })
            .collect();
        return json!({
            "kind": "native",
            "channels": channels,
            "newer": file.format > FILTER_FORMAT,
        });
    }
    let file = read_rew(text);
    let bands = file.filters.len().min(BANDS_PER_CHANNEL);
    let mut channels = Vec::new();
    if !file.filters.is_empty() || file.preamp.is_some() {
        for i in 0..inputs.min(n_in) {
            channels.push(choice(s, i, bands, true));
        }
    }
    if !file.filters.is_empty() {
        for o in 0..s.num_output_channels {
            channels.push(choice(s, n_in.saturating_add(o), bands, false));
        }
    }
    json!({
        "kind": "rew",
        "channels": channels,
        "filters": file.filters.len(),
        "preamp": file.preamp,
    })
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Every channel's bands as a filter file (format 2), from the mirror:
    /// the first `inputs` inputs with their preamp and EQ bands, then every
    /// output with its EQ and crossover bands and its enable state.
    ///
    /// Errors: as [`DspiCore::config_export`], when the mirror holds no
    /// device's state.
    pub fn filters_export(&self, inputs: u8) -> Result<String> {
        let (state, _) = exportable(&mut self.lock())?;
        Ok(filter_text(&state, inputs, unix_now()))
    }

    /// What a filter file holds, for the channel picker, as JSON; never
    /// fails. A Console file (macOS, Windows or this one) gives
    /// `{kind: "native", channels, newer}`: each channel it has a section
    /// for on this device, ticked, and whether a newer format wrote it. A
    /// REW or AutoEQ list gives `{kind: "rew", channels, filters, preamp}`:
    /// the first `inputs` inputs (ticked) and every output (not ticked), the
    /// number of filters and the preamp (null without one). A channel is
    /// `{wire, label, bands, checked}`. An empty `channels` means nothing in
    /// the file applies; a file over [`MAX_FILE_BYTES`] gives `kind`
    /// "error" with an `error`.
    pub fn filters_inspect(&self, text: String, inputs: u8) -> String {
        let state = self.lock().state.clone();
        inspect_filters(&state, &text, inputs).to_string()
    }

    /// Apply a filter file to the wire channels in `wires` (sections for
    /// other channels are ignored), through the band, preamp and output
    /// enable setters, under the device lock. A section's EQ or crossover
    /// lines replace that bank, the rest going flat; a section without lines
    /// for a bank leaves it alone, and the report says so. A REW list goes to
    /// every chosen channel, its preamp to the inputs only.
    ///
    /// Returns JSON `{ok, error, lines}`: `ok` false with `error` when the
    /// file is over [`MAX_FILE_BYTES`], nothing in it applies to the chosen
    /// channels, or the device could not be written (disconnected, read-only
    /// firmware, a failed transfer; channels written before a failure stay
    /// written).
    pub fn filters_import(&self, text: String, wires: Vec<u8>) -> String {
        if text.len() > MAX_FILE_BYTES {
            return json!({ "ok": false, "error": TOO_LARGE, "lines": [] }).to_string();
        }
        let lock = match self.acquire_device_lock(LOCK_REASON) {
            Ok(l) => l,
            Err(e) => {
                return json!({ "ok": false, "error": write_error(&e), "lines": [] }).to_string();
            }
        };
        let state = self.lock().state.clone();
        let plan = plan_filters(&state, &text, &wires);
        let reply = if plan.writes.is_empty() {
            json!({
                "ok": false,
                "error": "Nothing in the file applies to the chosen channels.",
                "lines": plan.notes,
            })
        } else {
            match self.run_filter_plan(&state, &plan) {
                Ok(lines) => json!({ "ok": true, "error": "", "lines": lines }),
                Err(e) => json!({ "ok": false, "error": write_error(&e), "lines": [] }),
            }
        };
        self.release_device_lock(lock);
        reply.to_string()
    }
}

impl DspiCore {
    /// Send a plan's writes; the report's lines.
    fn run_filter_plan(&self, s: &DspState, plan: &FilterPlan) -> Result<Vec<String>> {
        let mut notes = Vec::new();
        for w in &plan.writes {
            for (band, p) in (0u8..).zip(w.filters.iter().flatten()) {
                self.set_filter(w.wire, band, *p)?;
            }
            for (band, p) in (0u8..).zip(w.xover.iter().flatten()) {
                self.set_crossover(w.wire, band, *p)?;
            }
            if let Some(db) = w.preamp {
                self.set_input_preamp(w.wire, db)?;
            }
            if let Some((o, on)) = w.enable
                && self.set_output_enable(o, on)? != on
            {
                let name = s
                    .channel_names
                    .get(usize::from(w.wire))
                    .cloned()
                    .unwrap_or_default();
                notes.push(format!(
                    "{name} could not be enabled (conflicts with another output)."
                ));
            }
        }
        let n = plan.writes.len();
        let mut lines = vec![format!(
            "Filters imported to {n} channel{}.",
            if n == 1 { "" } else { "s" }
        )];
        lines.extend(plan.notes.iter().cloned());
        lines.extend(notes);
        Ok(lines)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg18;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_dl3;
