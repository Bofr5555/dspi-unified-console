//! Configuration and filter files.
//!
//! Two file formats move a tuning between devices and apps:
//!
//! - **`.dspipreset`** (schema 1): the JSON document the macOS and Windows
//!   Consoles, the Android app and the old desktop core share. It covers what
//!   a preset slot covers (EQ and crossovers, delays, gains, routing, the DSP
//!   features) and, in its own `io` block, the board's wiring. Values are
//!   stored raw, as the firmware holds them, and every field is read
//!   leniently (a missing or mistyped field takes its default), so files from
//!   either Console and from older builds load. Channels carry the Windows
//!   Console's channel id and, additively, the input / output index; the
//!   index wins when present. The crossfeed block carries the K340 fork's
//!   `pairMap` (Android patch 0002): always exported, imported only when
//!   present, and an invalid map falls back to the default pairing.
//! - **Filter text files** (format 2, after the macOS Console): a header,
//!   then one section per channel with its preamp (inputs), PEQ bands and
//!   crossover bands. The band grammar is a superset of REW's, so REW and
//!   AutoEQ exports import too, as one set of filters for the channels the
//!   user picks. The Windows Console's name-keyed sections ("Master L",
//!   "SPDIF 2 R", "PDM") and its crossover spelling (family, shape, slope)
//!   are read as well.
//!
//! This is a port of the old desktop core's `presetfile.rs` and
//! `filterfile.rs`, which the Qt app parses: the JSON shapes, the report
//! lines and the text written are the same. The session's `preset_file` and
//! `filterfile` write different text (typed serde documents, other number
//! precision), so they are not used here.
//!
//! # Trust boundary
//!
//! A file is untrusted input (PLAN §1.7). Parsing never panics; a document
//! or text over [`MAX_FILE_BYTES`] is refused before parsing; the filter
//! parser keeps at most one channel's worth of bands per section and only
//! sections for channels a device can have, so nothing in a file sizes an
//! allocation beyond its own length. Non-finite numbers take the field's
//! default.
//!
//! # Writing
//!
//! A configuration import builds the new state and writes it in one bulk
//! SET ([`DspiCore::apply_all_params`]); the hardware I/O fields that changed
//! are also sent one by one, since the firmware's bulk SET leaves pins and
//! limiters alone when the output config is device-global. A filter import
//! writes band by band through the parameter setters, as the old core did.
//!
//! Owned by WP C13 (docs/core-merge/PLAN.md).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use dspi_proto::generated::limiter;
use dspi_proto::xover;
use serde_json::{Map, Value, json};

use crate::connect::apply_all_params_locked;
use crate::consts::*;
use crate::device::{PREAMP_MAX_DB_RP2040, PREAMP_MAX_DB_RP2350};
use crate::records::*;
use crate::state::default_state;
use crate::{DspiCore, Result};

/// The `.dspipreset` schema this build reads and writes.
const SCHEMA_VERSION: i64 = 1;

/// The filter file format this build writes (and the newest it reads).
const FORMAT_VERSION: u32 = 2;

/// The largest file (bytes) [`config_inspect`], [`DspiCore::config_import`],
/// [`DspiCore::filters_inspect`] and [`DspiCore::filters_import`] accept. A
/// full export of a 17-channel device is about 100 KB.
pub const MAX_FILE_BYTES: usize = 4 << 20;

/// The line every successful import ends with.
const NOT_STORED: &str = "These changes are live but not yet stored on the device. Save them to a preset slot to keep them.";

const FILTER_FLAT: u8 = 0;
const FILTER_LINKWITZ_TRANSFORM: u8 = 11;
/// The last PEQ type (`HP1`).
const FILTER_PEQ_LAST: u8 = 13;

/// A flat band, as the firmware resets one.
fn flat() -> FilterParams {
    FilterParams {
        filter_type: FILTER_FLAT,
        bypass: false,
        freq: 1000.0,
        q: DEFAULT_Q,
        gain: 0.0,
        qp: DEFAULT_Q,
    }
}

fn is_crossover(t: u8) -> bool {
    (xover::XOVER_FIRST..=xover::XOVER_LAST).contains(&t)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

// ── Safe access to the record's vectors ───────────────────────────────

fn at<T: Copy>(v: &[T], i: usize, def: T) -> T {
    v.get(i).copied().unwrap_or(def)
}

fn at2<T: Copy>(v: &[Vec<T>], i: usize, j: usize, def: T) -> T {
    v.get(i).and_then(|r| r.get(j)).copied().unwrap_or(def)
}

fn put<T>(v: &mut [T], i: usize, x: T) {
    if let Some(d) = v.get_mut(i) {
        *d = x;
    }
}

fn put2<T>(v: &mut [Vec<T>], i: usize, j: usize, x: T) {
    if let Some(d) = v.get_mut(i).and_then(|r| r.get_mut(j)) {
        *d = x;
    }
}

fn row<T>(v: &[Vec<T>], i: usize) -> &[T] {
    v.get(i).map(Vec::as_slice).unwrap_or(&[])
}

fn name_at(s: &DspState, w: usize) -> &str {
    s.channel_names.get(w).map(String::as_str).unwrap_or("")
}

fn is_rp2350(s: &DspState) -> bool {
    s.platform_id == 1
}

/// The preamp range (dB) the chip applies correctly (`preamp_max_db`).
fn preamp_range(s: &DspState) -> (f32, f32) {
    let max = if is_rp2350(s) {
        PREAMP_MAX_DB_RP2350
    } else {
        PREAMP_MAX_DB_RP2040
    };
    (-60.0, max)
}

/// A valid pair map as the firmware stores it: explicit default bytes
/// (`(2p+1) << 4 | 2p`) become 0.
fn normalize_pair_map(map: &[u8]) -> Vec<u8> {
    (0u8..)
        .zip(map.iter().take(CROSSFEED_MAP_PAIRS))
        .map(|(p, &b)| {
            // p < 4, so 2p + 1 < 8 and (2p + 1) * 16 fits a u8.
            let l = p.saturating_mul(2);
            let default = l.saturating_add(1).saturating_mul(16) | l;
            if b == default { 0 } else { b }
        })
        .collect()
}

/// A name as the image can hold it: cut at the first NUL, then to
/// `CHANNEL_NAME_LEN - 1` bytes on a character boundary.
fn clip_name(name: &str) -> String {
    let name = name.split('\0').next().unwrap_or("");
    let max = CHANNEL_NAME_LEN.saturating_sub(1);
    let end = (0..=name.len().min(max))
        .rev()
        .find(|&l| name.is_char_boundary(l))
        .unwrap_or(0);
    name.get(..end).unwrap_or("").to_string()
}

/// ISO-8601 UTC for now (whole seconds), without a date library.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    utc_string(secs)
}

/// ISO-8601 UTC for `secs` since 1970, clamped to the year 9999.
// Howard Hinnant's civil-from-days algorithm. `secs` is clamped to
// 0..=253_402_300_799 (9999-12-31T23:59:59Z) first, so every intermediate is
// a small non-negative i64: nothing below can overflow or divide by zero.
#[allow(clippy::arithmetic_side_effects)]
fn utc_string(secs: u64) -> String {
    let secs = secs.min(253_402_300_799) as i64;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// The text without a leading UTF-8 byte-order mark (Notepad and
/// PowerShell write one).
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn too_large(what: &str) -> String {
    format!(
        "Not a valid {what}: the file is larger than {} MB.",
        MAX_FILE_BYTES >> 20
    )
}

// ═══════════════════════════════════════════════════════════════════════
// Configuration documents (.dspipreset)
// ═══════════════════════════════════════════════════════════════════════

/// Windows Console channel ids: inputs 0-1 and 11-16, outputs from 2, and
/// the RP2350 PDM output at 10.
const EXTRA_INPUT_BASE: i64 = 11;
const OUTPUT_BASE: i64 = 2;
const PDM_ID_RP2350: i64 = 10;

fn platform_name(id: u8) -> &'static str {
    if id == 1 { "RP2350" } else { "RP2040" }
}

fn id_for_input(i: usize) -> i64 {
    let i = i64::try_from(i).unwrap_or(i64::MAX);
    if i < 2 {
        i
    } else {
        EXTRA_INPUT_BASE.saturating_add(i).saturating_sub(2)
    }
}

fn id_for_output(o: usize, outputs: usize, rp2350: bool) -> i64 {
    if rp2350 && o.checked_add(1) == Some(outputs) {
        PDM_ID_RP2350
    } else {
        OUTPUT_BASE.saturating_add(i64::try_from(o).unwrap_or(i64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelRef {
    Input(usize),
    Output(usize),
}

fn ref_for_id(id: i64, outputs: usize, rp2350: bool) -> Option<ChannelRef> {
    if (0..2).contains(&id) {
        return usize::try_from(id).ok().map(ChannelRef::Input);
    }
    let extra = i64::try_from(MAX_INPUTS.saturating_sub(2)).unwrap_or(0);
    if (EXTRA_INPUT_BASE..EXTRA_INPUT_BASE.saturating_add(extra)).contains(&id) {
        let k = usize::try_from(id.saturating_sub(EXTRA_INPUT_BASE)).ok()?;
        return Some(ChannelRef::Input(k.saturating_add(2)));
    }
    if rp2350 && id == PDM_ID_RP2350 {
        return outputs.checked_sub(1).map(ChannelRef::Output);
    }
    let o = usize::try_from(id.checked_sub(OUTPUT_BASE)?).ok()?;
    (o < outputs).then_some(ChannelRef::Output(o))
}

// ── Lenient reading ──

fn num(v: &Value, key: &str, def: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(def)
}

/// A float field; missing, mistyped or not finite as an f32 = `def`.
fn f(v: &Value, key: &str, def: f32) -> f32 {
    let x = num(v, key, f64::from(def)) as f32;
    if x.is_finite() { x } else { def }
}

fn as_int(x: &Value) -> Option<i64> {
    x.as_i64().or_else(|| x.as_f64().map(|f| f as i64))
}

fn int(v: &Value, key: &str, def: i64) -> i64 {
    v.get(key).and_then(as_int).unwrap_or(def)
}

fn flag(v: &Value, key: &str, def: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(def)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// An integer field that fits `T`; missing, mistyped or out of range =
/// `def` (a file value never wraps).
fn int_as<T: TryFrom<i64> + Into<i64> + Copy>(v: &Value, key: &str, def: T) -> T {
    T::try_from(int(v, key, def.into())).unwrap_or(def)
}

/// `x` if it fits `T`, else `def`.
fn fit<T: TryFrom<i64>>(x: i64, def: T) -> T {
    T::try_from(x).unwrap_or(def)
}

/// An integer array, at most `max` entries (the rest is never read).
fn ints(v: &Value, key: &str, max: usize) -> Option<Vec<i64>> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().take(max).map(|x| as_int(x).unwrap_or(0)).collect())
}

/// A float array, at most `max` entries; a non-number or non-finite entry
/// is 0.
fn floats(v: &Value, key: &str, max: usize) -> Option<Vec<f32>> {
    v.get(key).and_then(Value::as_array).map(|a| {
        a.iter()
            .take(max)
            .map(|x| {
                let f = x.as_f64().unwrap_or(0.0) as f32;
                if f.is_finite() { f } else { 0.0 }
            })
            .collect()
    })
}

/// A block that is an object (missing / null / wrong type = None).
fn block<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|b| b.is_object())
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn band_json(p: &FilterParams) -> Value {
    json!({ "type": p.filter_type, "freqHz": p.freq, "q": p.q, "gain": p.gain, "qp": p.qp, "bypass": p.bypass })
}

/// A band from a document, and whether its type exists (an unknown type
/// reads as flat).
fn band_from(v: &Value, xover: bool) -> (FilterParams, bool) {
    let t = int(v, "type", 0);
    let valid = t == 0
        || if xover {
            (i64::from(xover::XOVER_FIRST)..=i64::from(xover::XOVER_LAST)).contains(&t)
        } else {
            (0..=i64::from(FILTER_PEQ_LAST)).contains(&t)
        };
    let p = FilterParams {
        filter_type: if valid {
            u8::try_from(t).unwrap_or(FILTER_FLAT)
        } else {
            FILTER_FLAT
        },
        bypass: flag(v, "bypass", false),
        freq: f(v, "freqHz", 1000.0),
        q: f(v, "q", DEFAULT_Q),
        gain: f(v, "gain", 0.0),
        qp: f(v, "qp", DEFAULT_Q),
    };
    (p, valid)
}

// ── Export ──

/// The device's configuration as a document. `links` is the app's
/// input-pair link mask (bit p = pair p), which the firmware does not hold.
fn export_config(s: &DspState, name: &str, app_version: &str, links: u8) -> String {
    let rp2350 = is_rp2350(s);
    let n_in = usize::from(s.num_input_channels);
    let n_out = usize::from(s.num_output_channels);
    let mut fw = format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch);
    if s.fw_beta > 0 && s.fw_beta != FW_BETA_EARLY {
        fw.push_str(&format!("-beta{}", s.fw_beta));
    }

    let bands = |w: usize| row(&s.filters, w).iter().map(band_json).collect::<Vec<_>>();
    let mut channels = Vec::new();
    for i in 0..n_in {
        channels.push(json!({
            "channelId": id_for_input(i), "name": name_at(s, i), "isOutput": false,
            "eqChannel": i, "inputIndex": i,
            "delayMs": at(&s.channel_delays, i, 0.0), "gainDb": 0.0, "muted": false, "enabled": true,
            "eq": bands(i), "crossover": [],
        }));
    }
    for o in 0..n_out {
        let w = n_in.saturating_add(o);
        channels.push(json!({
            "channelId": id_for_output(o, n_out, rp2350), "name": name_at(s, w), "isOutput": true,
            "eqChannel": w, "outputIndex": o,
            "delayMs": at(&s.channel_delays, w, 0.0), "gainDb": at(&s.output_gain_db, o, 0.0),
            "muted": at(&s.output_muted, o, false), "enabled": at(&s.output_enabled, o, false),
            "outputDelayMs": at(&s.output_delay_ms, o, 0.0),
            "limiter": {
                "enabled": at(&s.limiter_enabled, o, false),
                "thresholdDb": at(&s.limiter_threshold_db, o, 0.0),
                "releaseMs": at(&s.limiter_release_ms, o, 0.0),
                "linkGroup": at(&s.limiter_link_group, o, 0),
            },
            "eq": bands(w),
            "crossover": row(&s.xover, w).iter().map(band_json).collect::<Vec<_>>(),
        }));
    }
    let mut matrix = Vec::new();
    for i in 0..n_in.min(MAX_INPUTS) {
        for o in 0..n_out {
            matrix.push(json!({
                "input": i, "output": o, "enabled": at2(&s.matrix_routing, i, o, false),
                "invert": at2(&s.matrix_invert, i, o, false), "gainDb": at2(&s.matrix_gain, i, o, 0.0),
            }));
        }
    }

    let u = |k: u8| at(&s.upmix, usize::from(k), 0.0);
    let sh = |k: u8| at(&s.subharm, usize::from(k), 0.0);
    let t = |k: usize| at(&s.tube, k, 0.0);
    let mut doc = Map::new();
    doc.insert("schemaVersion".into(), json!(SCHEMA_VERSION));
    doc.insert(
        "meta".into(),
        json!({
            "name": name, "savedUtc": utc_now(), "appVersion": app_version,
            "platform": platform_name(s.platform_id), "firmwareVersion": fw,
            "wireFormatVersion": s.format_version, "inputChannelCount": n_in, "outputChannelCount": n_out,
            "masterVolumeMode": s.master_volume_mode, "outputConfigMode": s.output_config_mode,
        }),
    );
    let linked: Vec<bool> = [1u8, 2, 4, 8].iter().map(|b| links & b != 0).collect();
    doc.insert(
        "global".into(),
        json!({
            "inputPreampsDb": s.input_preamp_db, "bypass": s.bypass,
            "masterVolumeDb": s.master_volume_db, "userVolumeDb": s.user_volume_db,
            "inputSource": s.input_source, "lgSoundSyncEnabled": s.lg_sound_sync_enabled,
            "inputPairLinked": linked,
        }),
    );
    doc.insert(
        "loudness".into(),
        json!({
            "enabled": s.loudness_enabled, "refSpl": s.loudness_ref_spl,
            "intensityPct": s.loudness_intensity, "outputMask": s.loudness_output_mask,
        }),
    );
    doc.insert(
        "crossfeed".into(),
        json!({
            "enabled": s.crossfeed_enabled, "preset": s.crossfeed_preset, "freqHz": s.crossfeed_freq,
            "feedDb": s.crossfeed_feed, "itd": s.crossfeed_itd, "outputPairMask": s.crossfeed_output_pair_mask,
            "pairMap": s.crossfeed_pair_map,
        }),
    );
    doc.insert(
        "leveller".into(),
        json!({
            "enabled": s.leveller_enabled, "speed": s.leveller_speed, "lookahead": s.leveller_lookahead,
            "amountPct": s.leveller_amount, "maxGainDb": s.leveller_max_gain_db, "gateDb": s.leveller_gate_db,
            "detectorMask": s.leveller_detector_mask, "applyMask": s.leveller_apply_mask,
        }),
    );
    doc.insert(
        "psybass".into(),
        json!({
            "enabled": s.psybass_enabled, "cutoffHz": s.psybass_cutoff_hz, "harmonicsDb": s.psybass_harmonics_db,
            "driveDb": s.psybass_drive_db, "characterPct": s.psybass_character_pct,
            "originalDb": s.psybass_original_db, "outputMask": s.psybass_output_mask,
        }),
    );
    doc.insert(
        "upmix".into(),
        if rp2350 {
            json!({
                "enabled": u(UPMIX_PARAM_ENABLED) != 0.0,
                "centerMode": u(UPMIX_PARAM_CENTER_MODE) as i64,
                "surroundMode": u(UPMIX_PARAM_SURROUND_MODE) as i64,
                "strengthPct": u(UPMIX_PARAM_STRENGTH), "centerWidthPct": u(UPMIX_PARAM_WIDTH),
                "thresholdPct": u(UPMIX_PARAM_THRESHOLD), "attackMs": u(UPMIX_PARAM_ATTACK),
                "releaseMs": u(UPMIX_PARAM_RELEASE), "detectorHpfHz": u(UPMIX_PARAM_DETECTOR_HPF),
                "surroundDelayMs": u(UPMIX_PARAM_SURROUND_DELAY), "surroundHpfHz": u(UPMIX_PARAM_SURROUND_HPF),
                "surroundLpfHz": u(UPMIX_PARAM_SURROUND_LPF), "decorrPct": u(UPMIX_PARAM_DECORRELATION),
                "presenceDb": u(UPMIX_PARAM_PRESENCE),
            })
        } else {
            Value::Null
        },
    );
    doc.insert(
        "subharm".into(),
        json!({
            "enabled": sh(SUBHARM_PARAM_ENABLED) != 0.0, "outputMask": sh(SUBHARM_PARAM_MASK) as i64,
            "lowDb": sh(SUBHARM_PARAM_LOW), "highDb": sh(SUBHARM_PARAM_HIGH), "topDb": sh(SUBHARM_PARAM_TOP),
            "boostDb": sh(SUBHARM_PARAM_BOOST), "selectMode": sh(SUBHARM_PARAM_SELECT) as i64,
            "selectDepthPct": sh(SUBHARM_PARAM_DEPTH), "selectHoldMs": sh(SUBHARM_PARAM_HOLD),
            "ceilingDb": sh(SUBHARM_PARAM_CEILING), "linkPairs": sh(SUBHARM_PARAM_LINK) != 0.0,
        }),
    );
    doc.insert(
        "tube".into(),
        json!({
            "enabled": t(0) != 0.0, "outputMask": t(1) as i64, "tubeType": t(2) as i64, "driveDb": t(3),
            "biasPct": t(4), "asymDb": t(5), "hardnessPct": t(6), "sagPct": t(7), "rectifier": t(8) as i64,
            "xfmrEnabled": t(9) != 0.0, "xfmrDamping": t(10), "xfmrResHz": t(11), "mixPct": t(12), "trimDb": t(13),
        }),
    );
    doc.insert("channels".into(), Value::Array(channels));
    doc.insert("matrix".into(), Value::Array(matrix));
    let rate_hz = [44_100u32, 48_000, 96_000]
        .get(usize::from(s.i2s_input_rate.min(2)))
        .copied()
        .unwrap_or(48_000);
    let pins = usize::from(s.num_pin_outputs).clamp(1, MAX_PHYSICAL_OUTPUTS);
    let mut io = json!({
        "outputPins": s.output_pins.get(..pins).unwrap_or(&s.output_pins),
        "outputSlotTypes": s.output_types,
        "i2sBckPin": s.i2s_bck_pin, "mckEnabled": s.mck_enabled, "mckPin": s.mck_pin,
        "mckMultiplier": if s.mck_multiplier == 1 { 256 } else { 128 },
        "i2sClockMode": s.i2s_clock_mode, "i2sClockPinMode": s.i2s_clock_pin_mode, "i2sBckPinSlave": s.i2s_bck_pin_slave,
        "spdifRxPins": s.spdif_rx_pins.iter().take(3).collect::<Vec<_>>(),
        "spdifRxPin4": at(&s.spdif_rx_pins, 3, 0),
        "spdifEnabledExt": (s.spdif_inputs_enabled & 0x0E) / 2,
        "i2sRxPins": s.i2s_rx_pins, "i2sInputChannels": s.i2s_input_channels,
        "i2sInputRateHz": rate_hz,
        "adatEnabled": s.adat_out_enabled, "adatPin": s.adat_out_pin,
        "adatInputEnabled": s.adat_input_enabled, "adatInputPin": s.adat_input_pin,
        "adatInputClockMode": s.adat_input_clock_mode, "dacHwMute": Value::Null,
    });
    if s.dac_mute_supported
        && let Some(o) = io.as_object_mut()
    {
        o.insert(
            "dacHwMute".into(),
            json!({ "enabled": s.dac_mute_enabled, "activeLow": s.dac_mute_active_low,
                    "pin": s.dac_mute_pin, "holdMs": s.dac_mute_hold_ms, "releaseMs": s.dac_mute_release_ms }),
        );
    }
    doc.insert("io".into(), io);
    // serde_json's map is sorted, so two exports of one state diff cleanly.
    serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default()
}

// ── Import ──

/// Parse a document; Err is the message to show.
fn parse_config(text: &str) -> std::result::Result<Value, String> {
    if text.len() > MAX_FILE_BYTES {
        return Err(too_large("configuration file"));
    }
    let text = strip_bom(text);
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("Not a valid configuration file: {e}."))?;
    let version = int(&v, "schemaVersion", 0);
    if version <= 0 || array(&v, "channels").is_empty() {
        return Err("Not a valid configuration file: no channel data found.".into());
    }
    if version > SCHEMA_VERSION {
        return Err(format!(
            "This configuration was written by a newer version of DSPi Console (format {version}, this build reads {SCHEMA_VERSION})."
        ));
    }
    Ok(v)
}

/// Channels of a document this device does not have: each label once, the
/// first [`Missing::SHOWN`] named, the rest counted.
#[derive(Debug, Default)]
struct Missing {
    names: Vec<String>,
    /// Labels past the first `SHOWN` (bounded by the document's size).
    rest: BTreeSet<String>,
    more: usize,
}

impl Missing {
    const SHOWN: usize = 20;

    fn add(&mut self, s: String) {
        if self.names.contains(&s) {
            return;
        }
        if self.names.len() < Self::SHOWN {
            self.names.push(s);
        } else if self.rest.insert(s) {
            self.more = self.more.saturating_add(1);
        }
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// "a, b and 3 more".
    fn line(&self) -> String {
        let mut l = self.names.join(", ");
        if self.more > 0 {
            l.push_str(&format!(" and {} more", self.more));
        }
        l
    }
}

/// Things an import left out, each named once, in order.
#[derive(Debug, Default)]
struct Skips(Vec<String>);

impl Skips {
    fn add(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.0.contains(&s) {
            self.0.push(s);
        }
    }
}

/// A document applied to a copy of the mirror (nothing sent yet).
#[derive(Debug)]
struct ConfigBuild {
    state: DspState,
    links: Option<u8>,
    missing: Missing,
    skipped: Skips,
    channels: usize,
    bands: usize,
    xover: usize,
    points: usize,
}

/// Apply a parsed document to `old` (the mirror): the state to write and
/// what to report. Pure.
fn build_config(old: &DspState, v: &Value, options: u32) -> ConfigBuild {
    let mut ns = old.clone();
    let mut skipped = Skips::default();
    let links = build_global(&mut ns, v, options);
    build_features(&mut ns, v, &mut skipped);
    let mut b = ConfigBuild {
        state: ns,
        links,
        missing: Missing::default(),
        skipped,
        channels: 0,
        bands: 0,
        xover: 0,
        points: 0,
    };
    build_channels(&mut b, v, options & IMPORT_HARDWARE != 0);
    build_matrix(&mut b, v);
    if options & IMPORT_HARDWARE != 0
        && let Some(io) = block(v, "io")
    {
        build_io(&mut b.state, io);
    }
    b
}

/// The global block; returns the input-pair link mask, if present.
fn build_global(ns: &mut DspState, v: &Value, options: u32) -> Option<u8> {
    let g = v.get("global").unwrap_or(&Value::Null);
    if let Some(pre) = floats(g, "inputPreampsDb", MAX_INPUTS) {
        let (lo, hi) = preamp_range(ns);
        for (i, db) in pre.iter().enumerate() {
            put(&mut ns.input_preamp_db, i, db.clamp(lo, hi));
        }
        ns.preamp_db = at(&ns.input_preamp_db, 0, ns.preamp_db);
    }
    ns.bypass = flag(g, "bypass", ns.bypass);
    ns.input_source = int(g, "inputSource", i64::from(ns.input_source)).clamp(0, 6) as u8;
    ns.lg_sound_sync_enabled = flag(g, "lgSoundSyncEnabled", ns.lg_sound_sync_enabled);
    if options & IMPORT_VOLUMES != 0 {
        ns.master_volume_db = f(g, "masterVolumeDb", ns.master_volume_db).clamp(-128.0, 0.0);
        ns.user_volume_db = f(g, "userVolumeDb", ns.user_volume_db).clamp(-128.0, 0.0);
    }
    g.get("inputPairLinked").and_then(Value::as_array).map(|a| {
        a.iter()
            .zip([1u8, 2, 4, 8])
            .filter(|(x, _)| x.as_bool() == Some(true))
            .fold(0u8, |m, (_, bit)| m | bit)
    })
}

/// The DSP feature blocks.
fn build_features(ns: &mut DspState, v: &Value, skipped: &mut Skips) {
    if let Some(b) = block(v, "loudness") {
        ns.loudness_enabled = flag(b, "enabled", false);
        ns.loudness_ref_spl = f(b, "refSpl", 83.0);
        ns.loudness_intensity = f(b, "intensityPct", 100.0);
        ns.loudness_output_mask = int_as(b, "outputMask", ns.loudness_output_mask);
    }
    if let Some(b) = block(v, "crossfeed") {
        ns.crossfeed_enabled = flag(b, "enabled", false);
        ns.crossfeed_preset = int(b, "preset", 0).clamp(0, 3) as u8;
        ns.crossfeed_freq = f(b, "freqHz", 700.0);
        ns.crossfeed_feed = f(b, "feedDb", 4.5);
        ns.crossfeed_itd = flag(b, "itd", true);
        ns.crossfeed_output_pair_mask = int_as(b, "outputPairMask", ns.crossfeed_output_pair_mask);
        // K340 fork: absent (other clients) keeps the current map; an
        // invalid one (e.g. from a wider platform) falls back to default.
        if let Some(m) = ints(b, "pairMap", CROSSFEED_MAP_PAIRS) {
            let mut map = vec![0u8; CROSSFEED_MAP_PAIRS];
            for (dst, x) in map.iter_mut().zip(&m) {
                *dst = (*x).clamp(0, 255) as u8;
            }
            ns.crossfeed_pair_map =
                if crate::crossfeed_pair_map_valid(map.clone(), crate::ext::crossfeed_pairs(ns)) {
                    normalize_pair_map(&map)
                } else {
                    vec![0; CROSSFEED_MAP_PAIRS]
                };
        }
    }
    if let Some(b) = block(v, "leveller") {
        ns.leveller_enabled = flag(b, "enabled", false);
        ns.leveller_speed = int(b, "speed", 0).clamp(0, 2) as u8;
        ns.leveller_lookahead = flag(b, "lookahead", true);
        ns.leveller_amount = f(b, "amountPct", 50.0);
        ns.leveller_max_gain_db = f(b, "maxGainDb", 15.0);
        ns.leveller_gate_db = f(b, "gateDb", -96.0);
        ns.leveller_detector_mask = int_as(b, "detectorMask", 0xFF);
        ns.leveller_apply_mask = int_as(b, "applyMask", 0xFF);
    }
    if let Some(b) = block(v, "psybass") {
        ns.psybass_enabled = flag(b, "enabled", false);
        ns.psybass_cutoff_hz = f(b, "cutoffHz", 80.0);
        ns.psybass_harmonics_db = f(b, "harmonicsDb", 0.0);
        ns.psybass_drive_db = f(b, "driveDb", 6.0);
        ns.psybass_character_pct = f(b, "characterPct", 50.0);
        ns.psybass_original_db = f(b, "originalDb", 0.0);
        ns.psybass_output_mask = int_as(b, "outputMask", ns.psybass_output_mask);
    }
    let k = usize::from;
    if let Some(b) = block(v, "upmix") {
        if is_rp2350(ns) {
            let u = &mut ns.upmix;
            put(u, 0, f32::from(u8::from(flag(b, "enabled", false))));
            for (id, key) in [
                (UPMIX_PARAM_CENTER_MODE, "centerMode"),
                (UPMIX_PARAM_SURROUND_MODE, "surroundMode"),
            ] {
                let x = int(b, key, at(u, k(id), 0.0) as i64) as f32;
                put(u, k(id), x);
            }
            for (id, key) in [
                (UPMIX_PARAM_STRENGTH, "strengthPct"),
                (UPMIX_PARAM_WIDTH, "centerWidthPct"),
                (UPMIX_PARAM_THRESHOLD, "thresholdPct"),
                (UPMIX_PARAM_ATTACK, "attackMs"),
                (UPMIX_PARAM_RELEASE, "releaseMs"),
                (UPMIX_PARAM_DETECTOR_HPF, "detectorHpfHz"),
                (UPMIX_PARAM_SURROUND_DELAY, "surroundDelayMs"),
                (UPMIX_PARAM_SURROUND_HPF, "surroundHpfHz"),
                (UPMIX_PARAM_SURROUND_LPF, "surroundLpfHz"),
                (UPMIX_PARAM_DECORRELATION, "decorrPct"),
                (UPMIX_PARAM_PRESENCE, "presenceDb"),
            ] {
                let x = f(b, key, at(u, k(id), 0.0));
                put(u, k(id), x);
            }
        } else if flag(b, "enabled", false) {
            skipped.add("the stereo upmixer (not on RP2040)");
        }
    }
    if let Some(b) = block(v, "subharm") {
        let s = &mut ns.subharm;
        put(
            s,
            k(SUBHARM_PARAM_ENABLED),
            f32::from(u8::from(flag(b, "enabled", false))),
        );
        for (id, key) in [
            (SUBHARM_PARAM_MASK, "outputMask"),
            (SUBHARM_PARAM_SELECT, "selectMode"),
        ] {
            let x = int(b, key, at(s, k(id), 0.0) as i64) as f32;
            put(s, k(id), x);
        }
        for (id, key) in [
            (SUBHARM_PARAM_LOW, "lowDb"),
            (SUBHARM_PARAM_HIGH, "highDb"),
            (SUBHARM_PARAM_TOP, "topDb"),
            (SUBHARM_PARAM_BOOST, "boostDb"),
            (SUBHARM_PARAM_DEPTH, "selectDepthPct"),
            (SUBHARM_PARAM_HOLD, "selectHoldMs"),
            (SUBHARM_PARAM_CEILING, "ceilingDb"),
        ] {
            let x = f(b, key, at(s, k(id), 0.0));
            put(s, k(id), x);
        }
        put(
            s,
            k(SUBHARM_PARAM_LINK),
            f32::from(u8::from(flag(b, "linkPairs", true))),
        );
    }
    if let Some(b) = block(v, "tube") {
        let t = &mut ns.tube;
        put(t, 0, f32::from(u8::from(flag(b, "enabled", false))));
        let mask = int(b, "outputMask", at(t, 1, 0.0) as i64) as f32;
        put(t, 1, mask);
        let kind = int(b, "tubeType", at(t, 2, 0.0) as i64).clamp(0, 16) as f32;
        put(t, 2, kind);
        let rect = int(b, "rectifier", at(t, 8, 0.0) as i64).clamp(0, 3) as f32;
        put(t, 8, rect);
        let xfmr = flag(b, "xfmrEnabled", at(t, 9, 0.0) != 0.0);
        put(t, 9, f32::from(u8::from(xfmr)));
        for (i, key) in [
            (3, "driveDb"),
            (4, "biasPct"),
            (5, "asymDb"),
            (6, "hardnessPct"),
            (7, "sagPct"),
            (10, "xfmrDamping"),
            (11, "xfmrResHz"),
            (12, "mixPct"),
            (13, "trimDb"),
        ] {
            let x = f(b, key, at(t, i, 0.0));
            put(t, i, x);
        }
    }
}

/// The channel entries: names, delays, bands, output strips, limiters.
fn build_channels(b: &mut ConfigBuild, v: &Value, hardware: bool) {
    let ns = &mut b.state;
    let rp2350 = is_rp2350(ns);
    let n_in = usize::from(ns.num_input_channels);
    let n_out = usize::from(ns.num_output_channels);
    let max_delay = crate::state::max_delay_ms_for(ns);
    for c in array(v, "channels") {
        let index = |key: &str| {
            c.get(key)
                .and_then(Value::as_i64)
                // Negative: no such channel (reported missing).
                .map(|i| usize::try_from(i).unwrap_or(usize::MAX))
        };
        let r = match (
            flag(c, "isOutput", false),
            index("outputIndex"),
            index("inputIndex"),
        ) {
            (true, Some(o), _) => Some(ChannelRef::Output(o)),
            (false, _, Some(i)) => Some(ChannelRef::Input(i)),
            _ => ref_for_id(int(c, "channelId", -1), n_out, rp2350),
        };
        let w = match r {
            Some(ChannelRef::Input(i)) if i < n_in => i,
            Some(ChannelRef::Output(o)) if o < n_out => n_in.saturating_add(o),
            _ => {
                let label = text(c, "name")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("channel {}", int(c, "channelId", -1)));
                b.missing.add(label);
                continue;
            }
        };
        b.channels = b.channels.saturating_add(1);
        if let Some(name) = text(c, "name") {
            put(&mut ns.channel_names, w, clip_name(&name));
        }
        let delay = f(c, "delayMs", 0.0).clamp(0.0, max_delay);
        put(&mut ns.channel_delays, w, delay);
        let bands = array(c, "eq");
        if bands.len() > BANDS_PER_CHANNEL {
            b.skipped
                .add(format!("EQ bands beyond {BANDS_PER_CHANNEL} per channel"));
        }
        for k in 0..BANDS_PER_CHANNEL {
            let (p, ok) = bands.get(k).map_or((flat(), true), |x| band_from(x, false));
            if !ok {
                b.skipped
                    .add("filter types this firmware doesn't have (set to Off)");
            }
            if p.filter_type == FILTER_LINKWITZ_TRANSFORM && w < n_in {
                b.skipped.add("Linkwitz Transforms on inputs (set to Off)");
                put2(&mut ns.filters, w, k, flat());
                continue;
            }
            if p.filter_type != FILTER_FLAT {
                b.bands = b.bands.saturating_add(1);
            }
            put2(&mut ns.filters, w, k, p);
        }
        let Some(ChannelRef::Output(o)) = r else {
            continue;
        };
        let gain = f(c, "gainDb", 0.0).clamp(-60.0, 12.0);
        put(&mut ns.output_gain_db, o, gain);
        put(&mut ns.output_muted, o, flag(c, "muted", false));
        put(&mut ns.output_enabled, o, flag(c, "enabled", true));
        if c.get("outputDelayMs").is_some() {
            let d = f(c, "outputDelayMs", 0.0).clamp(0.0, max_delay);
            put(&mut ns.output_delay_ms, o, d);
        }
        let xs = array(c, "crossover");
        for k in 0..MAX_XOVER_BANDS {
            let (p, ok) = xs.get(k).map_or((flat(), true), |x| band_from(x, true));
            if !ok {
                b.skipped
                    .add("crossover types this firmware doesn't have (set to Off)");
            }
            if p.filter_type != FILTER_FLAT {
                b.xover = b.xover.saturating_add(1);
            }
            put2(&mut ns.xover, w, k, p);
        }
        if hardware && let Some(l) = block(c, "limiter") {
            put(&mut ns.limiter_enabled, o, flag(l, "enabled", false));
            let th = f(l, "thresholdDb", at(&ns.limiter_threshold_db, o, 0.0));
            put(&mut ns.limiter_threshold_db, o, th.clamp(-30.0, 0.0));
            let rel = f(l, "releaseMs", at(&ns.limiter_release_ms, o, 100.0));
            put(&mut ns.limiter_release_ms, o, rel.clamp(10.0, 1000.0));
            let group = int(l, "linkGroup", 0).clamp(0, 4) as u8;
            put(&mut ns.limiter_link_group, o, group);
        }
    }
}

/// Every crosspoint the document has.
fn build_matrix(b: &mut ConfigBuild, v: &Value) {
    let ns = &mut b.state;
    let n_in = usize::from(ns.num_input_channels).min(MAX_INPUTS);
    let n_out = usize::from(ns.num_output_channels);
    for x in array(v, "matrix") {
        let (Ok(i), Ok(o)) = (
            usize::try_from(int(x, "input", -1)),
            usize::try_from(int(x, "output", -1)),
        ) else {
            continue;
        };
        if i >= n_in || o >= n_out {
            continue;
        }
        let on = flag(x, "enabled", false);
        put2(&mut ns.matrix_routing, i, o, on);
        put2(&mut ns.matrix_invert, i, o, flag(x, "invert", false));
        put2(&mut ns.matrix_gain, i, o, f(x, "gainDb", 0.0));
        if on {
            b.points = b.points.saturating_add(1);
        }
    }
}

/// The hardware I/O block.
fn build_io(ns: &mut DspState, io: &Value) {
    let byte = |key: &str, def: u8| int_as(io, key, def);
    if let Some(pins) = ints(io, "outputPins", MAX_PHYSICAL_OUTPUTS) {
        for (k, p) in pins.iter().enumerate() {
            let x = fit(*p, at(&ns.output_pins, k, 0));
            put(&mut ns.output_pins, k, x);
        }
    }
    if let Some(types) = ints(io, "outputSlotTypes", 4) {
        for (k, t) in types.iter().enumerate() {
            put(&mut ns.output_types, k, (*t).clamp(0, 1) as u8);
        }
    }
    ns.i2s_bck_pin = byte("i2sBckPin", ns.i2s_bck_pin);
    ns.mck_enabled = flag(io, "mckEnabled", ns.mck_enabled);
    ns.mck_pin = byte("mckPin", ns.mck_pin);
    ns.mck_multiplier = u8::from(int(io, "mckMultiplier", 128) == 256);
    ns.i2s_clock_mode = int(io, "i2sClockMode", i64::from(ns.i2s_clock_mode)).clamp(0, 1) as u8;
    ns.i2s_clock_pin_mode =
        int(io, "i2sClockPinMode", i64::from(ns.i2s_clock_pin_mode)).clamp(0, 1) as u8;
    ns.i2s_bck_pin_slave = byte("i2sBckPinSlave", ns.i2s_bck_pin_slave);
    if let Some(rx) = ints(io, "spdifRxPins", 3) {
        for (k, p) in rx.iter().enumerate() {
            let x = fit(*p, at(&ns.spdif_rx_pins, k, 0));
            put(&mut ns.spdif_rx_pins, k, x);
        }
    }
    let pin4 = byte("spdifRxPin4", at(&ns.spdif_rx_pins, 3, 0));
    put(&mut ns.spdif_rx_pins, 3, pin4);
    // S/PDIF input 1 (bit 0) is always on.
    let ext = fit::<u8>(int(io, "spdifEnabledExt", 0), 0) & 0x07;
    ns.spdif_inputs_enabled = 0x01 | ext.saturating_mul(2);
    if let Some(rx) = ints(io, "i2sRxPins", 4) {
        for (k, p) in rx.iter().enumerate() {
            let x = fit(*p, at(&ns.i2s_rx_pins, k, 0));
            put(&mut ns.i2s_rx_pins, k, x);
        }
    }
    ns.i2s_input_channels = byte("i2sInputChannels", ns.i2s_input_channels);
    ns.i2s_input_rate = match int(io, "i2sInputRateHz", 48_000) {
        44_100 => 0,
        96_000 => 2,
        _ => 1,
    };
    if is_rp2350(ns) {
        ns.adat_out_enabled = flag(io, "adatEnabled", ns.adat_out_enabled);
        ns.adat_out_pin = byte("adatPin", ns.adat_out_pin);
        ns.adat_input_enabled = flag(io, "adatInputEnabled", ns.adat_input_enabled);
        ns.adat_input_pin = byte("adatInputPin", ns.adat_input_pin);
        ns.adat_input_clock_mode = int(
            io,
            "adatInputClockMode",
            i64::from(ns.adat_input_clock_mode),
        )
        .clamp(0, 1) as u8;
    }
    if let (Some(d), true) = (block(io, "dacHwMute"), ns.dac_mute_supported) {
        ns.dac_mute_enabled = flag(d, "enabled", false);
        ns.dac_mute_active_low = flag(d, "activeLow", true);
        ns.dac_mute_pin = int_as(d, "pin", ns.dac_mute_pin);
        ns.dac_mute_hold_ms = int_as(d, "holdMs", 0);
        ns.dac_mute_release_ms = int_as(d, "releaseMs", 0);
    }
}

/// The report lines of an import, and whether it was clean.
fn config_report(b: &ConfigBuild) -> (Vec<String>, bool) {
    let mut lines = vec![format!(
        "Applied {} channel{}, {} EQ band{}, {} crossover band{} and {} crosspoint{}.",
        b.channels,
        plural(b.channels),
        b.bands,
        plural(b.bands),
        b.xover,
        plural(b.xover),
        b.points,
        plural(b.points),
    )];
    if !b.missing.is_empty() {
        lines.push(format!("Not present on this device: {}.", b.missing.line()));
    }
    if !b.skipped.0.is_empty() {
        lines.push(format!("Skipped: {}.", b.skipped.0.join("; ")));
    }
    let clean = b.missing.is_empty() && b.skipped.0.is_empty();
    lines.push(NOT_STORED.into());
    (lines, clean)
}

/// Check a `.dspipreset` document and describe it, as JSON
/// `{ok, error, name, platform, firmware, savedUtc}` (`dspi_config_inspect`):
/// `ok` false with an `error` message for text that is not a document of
/// this schema (or is newer, or over [`MAX_FILE_BYTES`]). Needs no device.
pub fn config_inspect(text: String) -> String {
    match parse_config(&text) {
        Ok(v) => {
            let m = v.get("meta").unwrap_or(&Value::Null);
            let field = |k: &str| self::text(m, k).unwrap_or_default();
            json!({
                "ok": true,
                "name": field("name"),
                "platform": field("platform"),
                "firmware": field("firmwareVersion"),
                "savedUtc": field("savedUtc"),
            })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
    .to_string()
}

// ═══════════════════════════════════════════════════════════════════════
// Filter text files
// ═══════════════════════════════════════════════════════════════════════

fn uses_gain(t: u8) -> bool {
    matches!(t, 1 | 2 | 3 | 9 | 10)
}

fn uses_q(t: u8) -> bool {
    (1..=7).contains(&t)
}

/// The file token for a type (None = flat or unknown, written OFF).
fn file_code(t: u8) -> Option<String> {
    let code = match t {
        1 => "PK",
        2 => "LS",
        3 => "HS",
        4 => "LP",
        5 => "HP",
        6 => "NT",
        7 => "AP",
        8 => "AP1",
        9 => "LS1",
        10 => "HS1",
        11 => "LT",
        12 => "LP1",
        13 => "HP1",
        _ => {
            let m = xover::meta(t)?;
            return Some(format!(
                "{}{}{}",
                m.family.short().to_ascii_uppercase(),
                m.order,
                if m.high_pass { "HP" } else { "LP" }
            ));
        }
    };
    Some(code.to_string())
}

/// A crossover type from family, order and shape.
fn crossover_type(family: &str, order: u8, hp: bool) -> Option<u8> {
    let pair = match family {
        "LR" if matches!(order, 2 | 4 | 6 | 8) => (order / 2).checked_sub(1)?,
        "BW" if (1..=8).contains(&order) => order.checked_add(3)?,
        "BES" | "BESSEL" if matches!(order, 2 | 4 | 6 | 8) => (order / 2).checked_add(11)?,
        _ => return None,
    };
    xover::XOVER_FIRST
        .checked_add(pair.checked_mul(2)?)?
        .checked_add(u8::from(hp))
}

/// A type from a file token, with the aliases other tools write.
fn type_for_code(code: &str) -> Option<u8> {
    let c = code.to_ascii_uppercase();
    Some(match c.as_str() {
        "PK" | "PEQ" => 1,
        "LS" | "LSC" => 2,
        "HS" | "HSC" => 3,
        "LP" | "LPQ" => 4,
        "HP" | "HPQ" => 5,
        "NT" | "NO" => 6,
        "AP" => 7,
        "AP1" => 8,
        "LS1" => 9,
        "HS1" => 10,
        "LT" => 11,
        "LP1" => 12,
        "HP1" => 13,
        _ => {
            // "LR4LP", "BW3HP", "BES2LP"
            let (fam, rest) = ["BES", "LR", "BW"]
                .iter()
                .find_map(|f| c.strip_prefix(f).map(|r| (*f, r)))?;
            let (order, hp) = match (rest.strip_suffix("HP"), rest.strip_suffix("LP")) {
                (Some(o), _) => (o, true),
                (None, Some(o)) => (o, false),
                (None, None) => return None,
            };
            return crossover_type(fam, order.parse().ok()?, hp);
        }
    })
}

/// A number token, tolerating a glued unit and either decimal separator
/// (the last of '.' / ',' is the decimal point). Not finite = None.
fn number(token: &str) -> Option<f32> {
    let t = token.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != ',');
    if t.is_empty() {
        return None;
    }
    let mut t = t.to_string();
    if let (Some(d), Some(c)) = (t.rfind('.'), t.rfind(',')) {
        t = t.replace(if d > c { ',' } else { '.' }, "");
    }
    t.replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|x| x.is_finite())
}

fn format_band(xover: bool, index: usize, p: &FilterParams) -> String {
    let kw = if xover { "Xover " } else { "Filter" };
    let Some(code) = file_code(p.filter_type) else {
        return format!("{kw} {index:2}: OFF\n");
    };
    let mut line = format!("{kw} {index:2}: ON  {code:<8}Fc {:7.1} Hz", p.freq);
    if p.filter_type == FILTER_LINKWITZ_TRANSFORM {
        line.push_str(&format!(
            "  Q {:6.3}  Fp {:7.1} Hz  Qp {:6.3}",
            p.q, p.gain, p.qp
        ));
    } else {
        if uses_gain(p.filter_type) {
            line.push_str(&format!("  Gain {:+7.3} dB", p.gain));
        }
        if uses_q(p.filter_type) {
            line.push_str(&format!("  Q {:6.3}", p.q));
        }
    }
    if p.bypass {
        line.push_str("  [Bypassed]");
    }
    line.push('\n');
    line
}

/// Every channel's filters as a file. `inputs` = the live inputs to write;
/// `now` = the export time (Unix seconds).
fn export_filters(s: &DspState, inputs: usize, now: u64) -> String {
    let mut out = format!(
        "# DSPi Console Filter Settings\n# Exported: {now} (Unix time)\n# Format: {FORMAT_VERSION}\n\n"
    );
    let n_in = usize::from(s.num_input_channels);
    for i in 0..inputs.min(n_in) {
        out.push_str(&format!("[Input {i}: {}]\n", name_at(s, i)));
        out.push_str(&format!(
            "Preamp {:+.1} dB\n",
            at(&s.input_preamp_db, i, 0.0)
        ));
        for (b, p) in row(&s.filters, i).iter().enumerate() {
            out.push_str(&format_band(false, b.saturating_add(1), p));
        }
        out.push('\n');
    }
    for o in 0..usize::from(s.num_output_channels) {
        let w = n_in.saturating_add(o);
        let state = if at(&s.output_enabled, o, false) {
            "Enabled"
        } else {
            "Disabled"
        };
        out.push_str(&format!("[Output {o}: {} ({state})]\n", name_at(s, w)));
        for (b, p) in row(&s.filters, w).iter().enumerate() {
            out.push_str(&format_band(false, b.saturating_add(1), p));
        }
        for (b, p) in row(&s.xover, w).iter().enumerate() {
            out.push_str(&format_band(true, b.saturating_add(1), p));
        }
        out.push('\n');
    }
    out
}

// ── Reading ──

#[derive(Debug, Clone, PartialEq)]
struct Band {
    xover: bool,
    params: FilterParams,
    enabled: bool,
}

/// `s` starts with the ASCII keyword `kw`, ignoring ASCII case.
fn starts_with_ci(s: &str, kw: &str) -> bool {
    s.get(..kw.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(kw))
}

fn parse_preamp(line: &str) -> Option<f32> {
    let l = line.trim();
    if !starts_with_ci(l, "preamp") {
        return None;
    }
    l.get(6..)?
        .split(|c: char| c.is_whitespace() || c == ':')
        .find_map(number)
}

fn parse_band(line: &str) -> Option<Band> {
    let l = line.trim();
    let (kw, xover) = [("crossover", true), ("filter", false), ("xover", true)]
        .into_iter()
        .find(|(k, _)| starts_with_ci(l, k))?;
    let colon = l.find(':')?;
    if l.get(kw.len()..colon)?.trim().parse::<i32>().is_err() {
        return None;
    }
    let mut tokens = l.get(colon.saturating_add(1)..)?.split_whitespace();
    let off = Band {
        xover,
        params: flat(),
        enabled: false,
    };
    if !tokens.next().is_some_and(|t| t.eq_ignore_ascii_case("ON")) {
        return Some(off);
    }
    let Some(type_token) = tokens.next() else {
        return Some(off);
    };
    let tokens: Vec<&str> = tokens.collect();
    let mut values: BTreeMap<String, f32> = BTreeMap::new();
    for (k, v) in tokens.iter().zip(tokens.iter().skip(1)) {
        if number(k).is_none()
            && let Some(v) = number(v)
        {
            values.insert(k.to_ascii_uppercase(), v);
        }
    }
    // A crossover written as family + shape + slope (the Windows spelling).
    let t = type_for_code(type_token).or_else(|| {
        let slope = *values.get("SLOPE")?;
        let hp = match tokens.first()?.to_ascii_uppercase().as_str() {
            "HP" => true,
            "LP" => false,
            _ => return None,
        };
        crossover_type(&type_token.to_ascii_uppercase(), (slope / 6.0) as u8, hp)
    });
    let Some(t) = t else { return Some(off) };
    let mut p = FilterParams {
        filter_type: t,
        ..flat()
    };
    if let Some(v) = values.get("FC") {
        p.freq = *v;
    }
    if let Some(v) = values.get("GAIN") {
        p.gain = *v;
    }
    if let Some(v) = values.get("Q") {
        p.q = *v;
    }
    if let Some(v) = values.get("QP") {
        p.qp = *v;
    }
    if t == FILTER_LINKWITZ_TRANSFORM
        && let Some(v) = values.get("FP")
    {
        p.gain = *v;
    }
    p.bypass = l.to_ascii_uppercase().contains("[BYPASSED]")
        || tokens.iter().any(|t| t.eq_ignore_ascii_case("BYP"));
    Some(Band {
        xover,
        params: p,
        enabled: true,
    })
}

/// A channel at an input or output index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Chan {
    Input(usize),
    Output(usize),
}

/// One channel's section. Only the bands a channel can hold are kept; the
/// counts cover every line.
#[derive(Debug, Default, Clone)]
struct Section {
    filters: Vec<FilterParams>,
    xover: Vec<FilterParams>,
    /// PEQ band lines, on or off (the "first N bands" note).
    filter_lines: usize,
    /// Bands that are on, PEQ and crossover (the picker's count).
    active: usize,
    preamp: Option<f32>,
    enable: Option<bool>,
}

#[derive(Debug)]
enum Parsed {
    /// A DSPi Console file: per-channel sections.
    Native {
        channels: BTreeMap<Chan, Section>,
        version: u32,
    },
    /// A REW / AutoEQ list: one set of filters (the first
    /// `BANDS_PER_CHANNEL` kept, `count` in all).
    Rew {
        filters: Vec<FilterParams>,
        count: usize,
        preamp: Option<f32>,
        dropped: usize,
    },
}

/// The Windows Console's default channel names as section headers.
fn windows_header(name: &str, pdm: usize) -> Option<Chan> {
    let n = name.trim().to_ascii_uppercase();
    match n.as_str() {
        "MASTER L" | "USB L" => return Some(Chan::Input(0)),
        "MASTER R" | "USB R" => return Some(Chan::Input(1)),
        "PDM" | "SUB" => return Some(Chan::Output(pdm)),
        "OUT L" => return Some(Chan::Output(0)),
        "OUT R" => return Some(Chan::Output(1)),
        _ => {}
    }
    if let Some(i) = n
        .strip_prefix("INPUT ")
        .and_then(|x| x.parse::<usize>().ok())
    {
        return (3..=8)
            .contains(&i)
            .then(|| Chan::Input(i.saturating_sub(1)));
    }
    let rest = n.strip_prefix("SPDIF ")?;
    let mut parts = rest.split_whitespace();
    let pair: usize = parts.next()?.parse().ok()?;
    let left = pair.checked_sub(1)?.checked_mul(2)?;
    match parts.next()? {
        "L" => Some(Chan::Output(left)),
        "R" => Some(Chan::Output(left.checked_add(1)?)),
        _ => None,
    }
}

/// A section header: the channel and, for an output, its enable state.
fn section_header(h: &str, pdm: usize) -> Option<(Chan, Option<bool>)> {
    let idx = |prefix: &str| {
        h.strip_prefix(prefix)
            .and_then(|r| r.split(':').next())
            .and_then(|n| n.trim().parse::<usize>().ok())
    };
    // The native writer always emits `Input i: name`; a bare `Input 3` is
    // the Windows Console's 1-based name, read by `windows_header`.
    if !h.contains(':') {
        return windows_header(h, pdm).map(|c| (c, None));
    }
    if let Some(i) = idx("Input ") {
        return Some((Chan::Input(i), None));
    }
    if let Some(o) = idx("Output ") {
        let e = if h.ends_with("(Enabled)") {
            Some(true)
        } else if h.ends_with("(Disabled)") {
            Some(false)
        } else {
            None
        };
        return Some((Chan::Output(o), e));
    }
    windows_header(h, pdm).map(|c| (c, None))
}

fn parse_filters(text: &str, pdm: usize) -> Parsed {
    let text = strip_bom(text);
    if !text.trim_start().starts_with("# DSPi Console") {
        let mut filters = Vec::new();
        let (mut count, mut dropped) = (0usize, 0usize);
        let mut preamp = None;
        for line in text.lines() {
            if let Some(p) = parse_preamp(line) {
                preamp = Some(p);
            } else if let Some(b) = parse_band(line) {
                if b.enabled && !b.xover {
                    if filters.len() < BANDS_PER_CHANNEL {
                        filters.push(b.params);
                    }
                    count = count.saturating_add(1);
                } else {
                    dropped = dropped.saturating_add(1);
                }
            }
        }
        return Parsed::Rew {
            filters,
            count,
            preamp,
            dropped,
        };
    }
    let mut channels: BTreeMap<Chan, Section> = BTreeMap::new();
    let mut version = 1;
    let mut current: Option<Chan> = None;
    for line in text.lines() {
        let l = line.trim();
        if starts_with_ci(l, "# format") {
            version = l
                .split(|c: char| c.is_whitespace() || c == ':')
                .find_map(|t| t.parse().ok())
                .unwrap_or(1);
            continue;
        }
        if let Some(h) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            current = None;
            // Only channels a device can have get a section.
            let header = section_header(h, pdm).filter(|(c, _)| match *c {
                Chan::Input(i) => i < MAX_INPUTS,
                Chan::Output(o) => o < MAX_OUTPUTS,
            });
            if let Some((c, enable)) = header {
                channels.insert(
                    c,
                    Section {
                        enable,
                        ..Section::default()
                    },
                );
                current = Some(c);
            }
            continue;
        }
        let Some(sec) = current.and_then(|c| channels.get_mut(&c)) else {
            continue;
        };
        if let Some(p) = parse_preamp(l) {
            sec.preamp = Some(p);
        } else if let Some(b) = parse_band(l) {
            // OFF / unknown keeps its slot (flat) so indices stay aligned.
            let p = if b.enabled { b.params } else { flat() };
            if p.filter_type != FILTER_FLAT {
                sec.active = sec.active.saturating_add(1);
            }
            if b.xover {
                if sec.xover.len() < MAX_XOVER_BANDS {
                    sec.xover.push(p);
                }
            } else {
                sec.filter_lines = sec.filter_lines.saturating_add(1);
                if sec.filters.len() < BANDS_PER_CHANNEL {
                    sec.filters.push(p);
                }
            }
        }
    }
    Parsed::Native { channels, version }
}

/// The wire index of a file channel on this device, if it has it.
fn wire_of(c: Chan, n_in: usize, n_out: usize) -> Option<usize> {
    match c {
        Chan::Input(i) if i < n_in => Some(i),
        Chan::Output(o) if o < n_out => n_in.checked_add(o),
        _ => None,
    }
}

/// What a file holds, for the channel picker (JSON): a DSPi file lists its
/// channels (wire index, label, band count, all ticked); a REW file its
/// filter count and preamp, with every channel offered (inputs ticked).
fn inspect_filters(s: &DspState, text: &str, inputs: usize) -> Value {
    if text.len() > MAX_FILE_BYTES {
        return json!({ "kind": "error", "channels": [], "error": too_large("filter file") });
    }
    let n_in = usize::from(s.num_input_channels);
    let n_out = usize::from(s.num_output_channels);
    let label = |w: usize| {
        let name = name_at(s, w);
        if w < n_in {
            format!("Input {}: {name}", w.saturating_add(1))
        } else {
            format!(
                "Output {}: {name}",
                w.saturating_sub(n_in).saturating_add(1)
            )
        }
    };
    match parse_filters(text, n_out.saturating_sub(1)) {
        Parsed::Native { channels, version } => {
            let list: Vec<Value> = channels
                .iter()
                .filter_map(|(c, sec)| {
                    let w = wire_of(*c, n_in, n_out)?;
                    Some(json!({ "wire": w, "label": label(w), "bands": sec.active, "checked": true }))
                })
                .collect();
            json!({ "kind": "native", "channels": list, "newer": version > FORMAT_VERSION })
        }
        Parsed::Rew { count, preamp, .. } => {
            let mut list = Vec::new();
            for w in 0..inputs.min(n_in) {
                list.push(json!({ "wire": w, "label": label(w), "bands": count, "checked": true }));
            }
            for o in 0..n_out {
                let w = n_in.saturating_add(o);
                list.push(
                    json!({ "wire": w, "label": label(w), "bands": count, "checked": false }),
                );
            }
            json!({ "kind": "rew", "channels": list, "filters": count, "preamp": preamp })
        }
    }
}

/// What a filter import writes to one channel.
#[derive(Debug, Clone, PartialEq)]
struct ChannelWrite {
    wire: u8,
    /// Every PEQ band (`BANDS_PER_CHANNEL`), flat where the file has none.
    filters: Vec<FilterParams>,
    /// Every crossover band (`MAX_XOVER_BANDS`), outputs only.
    xover: Option<Vec<FilterParams>>,
    /// Inputs only.
    preamp: Option<f32>,
    /// (output index, enabled), outputs only.
    enable: Option<(u8, bool)>,
}

/// A filter import, planned (nothing sent yet).
#[derive(Debug, Default)]
struct FilterPlan {
    writes: Vec<ChannelWrite>,
    notes: Vec<String>,
}

impl FilterPlan {
    fn note(&mut self, n: String) {
        if !self.notes.contains(&n) {
            self.notes.push(n);
        }
    }
}

/// The bands, preamp and enable state written to wire channel `w`.
fn channel_write(
    w: usize,
    n_in: usize,
    filters: &[FilterParams],
    xover: Option<&[FilterParams]>,
    preamp: Option<f32>,
    enable: Option<bool>,
) -> Option<ChannelWrite> {
    let wire = u8::try_from(w).ok()?;
    let is_input = w < n_in;
    let filters = (0..BANDS_PER_CHANNEL)
        .map(|b| {
            let p = filters.get(b).copied().unwrap_or_else(flat);
            if p.filter_type > FILTER_PEQ_LAST
                || (p.filter_type == FILTER_LINKWITZ_TRANSFORM && is_input)
            {
                flat()
            } else {
                p
            }
        })
        .collect();
    let xover = xover.filter(|_| !is_input).map(|x| {
        (0..MAX_XOVER_BANDS)
            .map(|b| {
                let p = x.get(b).copied().unwrap_or_else(flat);
                if p.filter_type != FILTER_FLAT && !is_crossover(p.filter_type) {
                    flat()
                } else {
                    p
                }
            })
            .collect()
    });
    let output = w
        .checked_sub(n_in)
        .and_then(|o| u8::try_from(o).ok())
        .filter(|_| !is_input);
    Some(ChannelWrite {
        wire,
        filters,
        xover,
        preamp: preamp.filter(|_| is_input),
        enable: output.zip(enable),
    })
}

/// Plan a filter import onto the chosen wire channels of `s`. Pure.
fn plan_filters(s: &DspState, text: &str, chosen: &[u8]) -> FilterPlan {
    let n_in = usize::from(s.num_input_channels);
    let n_out = usize::from(s.num_output_channels);
    let mut plan = FilterPlan::default();
    match parse_filters(text, n_out.saturating_sub(1)) {
        Parsed::Native { channels, version } => {
            if version > FORMAT_VERSION {
                plan.note(format!(
                    "The file is format {version}; this build reads format {FORMAT_VERSION}, so newer sections were skipped."
                ));
            }
            for (c, sec) in &channels {
                let Some(w) = wire_of(*c, n_in, n_out) else {
                    continue;
                };
                if !chosen.iter().any(|&x| usize::from(x) == w) {
                    continue;
                }
                if sec.filter_lines > BANDS_PER_CHANNEL {
                    plan.note(format!(
                        "Only the first {BANDS_PER_CHANNEL} bands of a channel were used."
                    ));
                }
                let xover = Some(sec.xover.as_slice());
                if let Some(cw) =
                    channel_write(w, n_in, &sec.filters, xover, sec.preamp, sec.enable)
                {
                    plan.writes.push(cw);
                }
            }
        }
        Parsed::Rew {
            filters,
            count,
            preamp,
            dropped,
        } => {
            if count > BANDS_PER_CHANNEL {
                plan.note(format!(
                    "The file has {count} filters; only the first {BANDS_PER_CHANNEL} were used."
                ));
            }
            if dropped > 0 {
                plan.note(format!(
                    "{dropped} filter line{} that were off or of an unknown type were left out.",
                    plural(dropped)
                ));
            }
            if preamp.is_some() && chosen.iter().all(|&w| usize::from(w) >= n_in) {
                plan.note("The preamp applies to inputs only, so it was not used.".into());
            }
            for &w in chosen {
                let w = usize::from(w);
                if w < n_in.saturating_add(n_out)
                    && let Some(cw) = channel_write(w, n_in, &filters, None, preamp, None)
                {
                    plan.writes.push(cw);
                }
            }
        }
    }
    plan
}

fn filter_report(applied: usize, notes: &[String]) -> Vec<String> {
    let mut lines = vec![format!(
        "Applied filters to {applied} channel{}.",
        plural(applied)
    )];
    lines.extend(notes.iter().cloned());
    lines.push(NOT_STORED.into());
    lines
}

fn write_failed(e: &crate::DspiError) -> Value {
    json!({ "ok": false, "error": format!("The device could not be written: {e}.") })
}

// ═══════════════════════════════════════════════════════════════════════
// The API
// ═══════════════════════════════════════════════════════════════════════

/// What a configuration import did.
struct ImportReport {
    lines: Vec<String>,
    clean: bool,
    links: Option<u8>,
    hardware_written: bool,
}

impl DspiCore {
    /// The device configuration (the mirror; defaults while disconnected)
    /// as a `.dspipreset` document: schema 1, pretty-printed JSON with
    /// sorted keys. `name` and `app_version` go into `meta`; `links` = the
    /// app's input-pair link mask (bit p = pair p), which the firmware does
    /// not hold. Never fails.
    pub fn config_export(&self, name: String, app_version: String, links: u8) -> String {
        let inner = self.lock();
        export_config(&inner.state, &name, &app_version, links)
    }

    /// Apply a `.dspipreset` document to the device. `options`: bit
    /// [`IMPORT_VOLUMES`] also applies the master and user volume, bit
    /// [`IMPORT_HARDWARE`] the hardware I/O and the limiters.
    ///
    /// Returns JSON `{ok, error, lines, clean, links, hardwareWritten}`:
    /// `ok` false with `error` for a file that is not a document of this
    /// schema, or when the device could not be written (disconnected,
    /// read-only firmware, transfer failure; the mirror is then left as it
    /// was). On success `lines` is the report for the user, `clean` is false
    /// when channels or settings were left out, `links` the input-pair link
    /// mask to restore (null when the document has none) and
    /// `hardwareWritten` whether I/O was written outside a preset (save the
    /// output config to keep it).
    pub fn config_import(&self, text: String, options: u32) -> String {
        let out = match parse_config(&text) {
            Err(e) => json!({ "ok": false, "error": e }),
            Ok(v) => match self.import_config(&v, options) {
                Ok(r) => json!({
                    "ok": true, "lines": r.lines, "clean": r.clean, "links": r.links,
                    "hardwareWritten": r.hardware_written,
                }),
                Err(e) => write_failed(&e),
            },
        };
        out.to_string()
    }

    /// Build the new state, write it in one bulk SET, then send the
    /// hardware I/O that changed one setting at a time.
    fn import_config(&self, v: &Value, options: u32) -> Result<ImportReport> {
        // The Link device lock first, with the core lock released (WP C15).
        let device_lock = self.acquire_device_lock(crate::connect::LOCK_REASON)?;
        // Swap the mirror and write it in one critical section, so no
        // notification lands in between; restore it if the write fails.
        let swapped = (|| {
            let mut inner = self.lock();
            if !device_lock.is_current(&inner) {
                return Err(crate::DspiError::Disconnected);
            }
            // Refuse a read-only or missing device before touching the mirror.
            inner.writable_conn().map(|_| ())?;
            let old = std::mem::replace(&mut inner.state, default_state());
            let built = build_config(&old, v, options);
            inner.state = built.state.clone();
            // The device announces the bulk write with BULK_INVALIDATED,
            // which re-reads everything.
            if let Err(e) = apply_all_params_locked(&mut inner) {
                inner.state = old;
                return Err(e);
            }
            Ok((old, built))
        })();
        self.release_device_lock(device_lock);
        let (old, mut built) = swapped?;
        let hardware_written = if options & IMPORT_HARDWARE != 0 {
            let refused = self.write_io(&built.state, &old);
            for r in refused {
                built.skipped.add(format!("{r} (refused by the device)"));
            }
            old.output_config_mode == 0
        } else {
            false
        };
        let (lines, clean) = config_report(&built);
        Ok(ImportReport {
            lines,
            clean,
            links: built.links,
            hardware_written,
        })
    }

    /// Send the hardware I/O settings of `s` that differ from `old`, one at
    /// a time. Returns what the device refused.
    fn write_io(&self, s: &DspState, old: &DspState) -> Vec<String> {
        let mut refused = Vec::new();
        let mut check = |what: String, r: Result<()>| {
            if r.is_err() {
                refused.push(what);
            }
        };
        let changed = |a: &[u8], b: &[u8], k: usize| a.get(k) != b.get(k);
        for k in 0u8..4 {
            let i = usize::from(k);
            if changed(&s.output_types, &old.output_types, i) {
                let r = self.set_output_type(k, at(&s.output_types, i, 0));
                check(format!("output {} type", k.saturating_add(1)), r);
            }
        }
        let pins = usize::from(s.num_pin_outputs).min(MAX_PHYSICAL_OUTPUTS);
        for k in (0u8..).take(pins) {
            let i = usize::from(k);
            if changed(&s.output_pins, &old.output_pins, i) {
                let r = self.set_output_pin(k, at(&s.output_pins, i, 0));
                check(format!("output pin {}", k.saturating_add(1)), r);
            }
        }
        if s.i2s_clock_pin_mode != old.i2s_clock_pin_mode {
            let r = self.set_i2s_clock_pin_mode(s.i2s_clock_pin_mode);
            check("I2S clock pin mode".into(), r);
        }
        if s.i2s_bck_pin != old.i2s_bck_pin {
            let r = self.set_i2s_bck_pin(0, s.i2s_bck_pin);
            check("I2S BCK pin".into(), r);
        }
        if s.i2s_bck_pin_slave != old.i2s_bck_pin_slave {
            let r = self.set_i2s_bck_pin(1, s.i2s_bck_pin_slave);
            check("I2S slave BCK pin".into(), r);
        }
        if s.mck_pin != old.mck_pin {
            let r = self.set_mck_pin(s.mck_pin);
            check("MCK pin".into(), r);
        }
        if s.mck_multiplier != old.mck_multiplier {
            let r = self.set_mck_multiplier(s.mck_multiplier);
            check("MCK multiplier".into(), r);
        }
        if s.mck_enabled != old.mck_enabled {
            let r = self.set_mck_enabled(s.mck_enabled);
            check("MCK output".into(), r);
        }
        if is_rp2350(s) && s.adat_out_pin != old.adat_out_pin {
            let r = self.set_adat_out_pin(s.adat_out_pin);
            check("ADAT output pin".into(), r);
        }
        if is_rp2350(s) && s.adat_out_enabled != old.adat_out_enabled {
            let r = self.set_adat_out_enabled(s.adat_out_enabled);
            check("ADAT output".into(), r);
        }
        // Limiters: best effort, as in the old core.
        let param = |p: u16| u8::try_from(p).unwrap_or(u8::MAX);
        let outputs = usize::from(s.num_output_channels).min(MAX_OUTPUTS);
        for o in (0u8..).take(outputs) {
            let i = usize::from(o);
            let send = |p: u16, value: f32| {
                let _ = self.set_limiter_param(o, param(p), value);
            };
            if s.limiter_enabled.get(i) != old.limiter_enabled.get(i) {
                let on = f32::from(u8::from(at(&s.limiter_enabled, i, false)));
                send(limiter::LIMITER_PARAM_ENABLED, on);
            }
            if s.limiter_threshold_db.get(i) != old.limiter_threshold_db.get(i) {
                send(
                    limiter::LIMITER_PARAM_THRESHOLD_DB,
                    at(&s.limiter_threshold_db, i, 0.0),
                );
            }
            if s.limiter_release_ms.get(i) != old.limiter_release_ms.get(i) {
                send(
                    limiter::LIMITER_PARAM_RELEASE_MS,
                    at(&s.limiter_release_ms, i, 100.0),
                );
            }
            if s.limiter_link_group.get(i) != old.limiter_link_group.get(i) {
                let g = f32::from(at(&s.limiter_link_group, i, 0));
                send(limiter::LIMITER_PARAM_LINK_GROUP, g);
            }
        }
        refused
    }

    /// Every channel's filters as a filter text file (format 2): the first
    /// `inputs` live inputs with their preamp and PEQ bands, then every
    /// output with its PEQ and crossover bands and enable state. Reads the
    /// mirror; never fails.
    pub fn filters_export(&self, inputs: u8) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let inner = self.lock();
        export_filters(&inner.state, usize::from(inputs), now)
    }

    /// What a filter file holds, for the channel picker, as JSON.
    ///
    /// A DSPi Console file: `{kind: "native", channels: [{wire, label,
    /// bands, checked}], newer}` with the channels this device has (`newer`:
    /// written in a newer format). A REW / AutoEQ list: `{kind: "rew",
    /// channels, filters, preamp}` offering the first `inputs` inputs
    /// (ticked) and every output. An empty `channels` means nothing in the
    /// file applies (`kind` "error" with an `error` for a file over
    /// [`MAX_FILE_BYTES`]). Never fails.
    pub fn filters_inspect(&self, text: String, inputs: u8) -> String {
        // Parse outside the lock, on a copy of the mirror.
        let state = self.lock().state.clone();
        inspect_filters(&state, &text, usize::from(inputs)).to_string()
    }

    /// Apply a filter file to the channels in `wires` (wire indices; other
    /// channels in the file are ignored) through the band, preamp and
    /// output-enable setters. Returns JSON `{ok, error, lines}`: `ok` false
    /// with `error` when the file is over [`MAX_FILE_BYTES`] or the device
    /// could not be written (disconnected, read-only firmware, transfer
    /// failure; channels written before a failure stay written).
    pub fn filters_import(&self, text: String, wires: Vec<u8>) -> String {
        if text.len() > MAX_FILE_BYTES {
            return json!({ "ok": false, "error": too_large("filter file") }).to_string();
        }
        let state = {
            let mut inner = self.lock();
            if let Err(e) = inner.writable_conn() {
                return write_failed(&e).to_string();
            }
            inner.state.clone()
        };
        // Each wire once, parsed outside the lock.
        let wires: Vec<u8> = wires
            .into_iter()
            .collect::<BTreeSet<u8>>()
            .into_iter()
            .collect();
        let mut plan = plan_filters(&state, &text, &wires);
        let writes = std::mem::take(&mut plan.writes);
        for w in &writes {
            match self.write_channel(w) {
                Ok(Some(note)) => plan.note(note),
                Ok(None) => {}
                Err(e) => return write_failed(&e).to_string(),
            }
        }
        json!({ "ok": true, "lines": filter_report(writes.len(), &plan.notes) }).to_string()
    }

    /// Write one channel; `Some(note)` when the device kept the output in
    /// another enable state than the file asks for.
    fn write_channel(&self, w: &ChannelWrite) -> Result<Option<String>> {
        for (b, p) in (0u8..).zip(&w.filters) {
            self.set_filter(w.wire, b, *p)?;
        }
        if let Some(x) = &w.xover {
            for (b, p) in (0u8..).zip(x) {
                self.set_crossover(w.wire, b, *p)?;
            }
        }
        if let Some(db) = w.preamp {
            // Clamped to the chip's range by the setter.
            self.set_input_preamp(w.wire, db)?;
        }
        if let Some((o, on)) = w.enable
            && self.set_output_enable(o, on)? != on
        {
            let what = if on { "enabled" } else { "disabled" };
            return Ok(Some(format!(
                "Output {} could not be {what}.",
                o.saturating_add(1)
            )));
        }
        Ok(None)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c13;
