//! The `.dspipreset` whole-device document.
//!
//! Schema and semantics follow the Windows Console's `PresetDocument.cs` and
//! `PresetFileService.cs`, pinned at `DSPi-Console-Windows@81ae00b`. These files
//! are destined for the macOS Console too, so divergence would break
//! interchange rather than merely inconvenience it.
//!
//! The macOS Console's `PresetDocument.swift` and `PresetDocumentTransfer.swift`
//! (DSPi Console `9dbb07a`) are the reference for everything the Windows schema
//! lacks, and for how a channel entry is placed. `docs/preset-format.md` writes
//! the format down, including the channel numbering, which is the part most
//! easily got wrong: a channel entry's `channelId` is the Windows Console's
//! channel id, not the firmware's unified channel index. See [`channel_id`].
//!
//! Four decisions are inherited deliberately, because each is correct and each
//! would be tempting to get wrong:
//!
//! - **Raw wire values, not enum names.** Wire values are the firmware's own and
//!   stay stable; a renamed enum member would silently break old files.
//! - **Nullable capability blocks**, so a document from a device without a
//!   feature is distinguishable from one with the feature switched off.
//! - **A schema version that rejects documents from the future** rather than
//!   guessing at blocks it does not understand.
//! - **Three-way apply options**, because volume levels and hardware wiring
//!   describe a room and a board rather than a tuning, and importing them
//!   silently would change how loud someone's room gets.
//!
//! One decision is this crate's own (WP TM21), because a zero looks exactly like
//! a setting: **a key the document does not hold is a setting it does not
//! speak for.** Every scalar is an `Option`; [`capture`] omits what the device
//! will not report rather than writing a zero, and [`apply`] leaves an omitted
//! key alone and says so ([`ApplyReport::left_alone`]). The Console and Qt's
//! importer fill a missing key with a default instead, so they are not bound by
//! this; `docs/terminal/preset-format.md` says where they differ.

use serde::{Deserialize, Serialize};

/// Bumped when the layout changes incompatibly.
pub const CURRENT_SCHEMA_VERSION: i32 = 1;

pub const FILE_EXTENSION: &str = "dspipreset";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PresetFileError {
    #[error("not a valid preset file: {0}")]
    Invalid(String),

    #[error("this file is empty")]
    Empty,

    #[error("no channel data found")]
    NoChannels,

    #[error(
        "this file was written by a newer version (format {got}, this build reads {supported})"
    )]
    FromTheFuture { got: i32, supported: i32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDocument {
    pub schema_version: i32,
    #[serde(default)]
    pub meta: Meta,
    #[serde(default)]
    pub global: GlobalBlock,
    // A block with nothing in it is left out whole: Qt's importer keeps the
    // device's values for an absent block, and defaults the keys of a present one.
    #[serde(default, skip_serializing_if = "says_nothing")]
    pub loudness: LoudnessBlock,
    #[serde(default, skip_serializing_if = "says_nothing")]
    pub crossfeed: CrossfeedBlock,
    #[serde(default, skip_serializing_if = "says_nothing")]
    pub leveller: LevellerBlock,
    /// Absent when the source device had no psychoacoustic bass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psybass: Option<PsybassBlock>,
    /// Absent when the source device had no upmixer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upmix: Option<UpmixBlock>,
    /// Absent when the source device had no subharmonic synthesizer
    /// (PresetDocument.swift:45).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subharm: Option<SubharmBlock>,
    /// Absent when the source device had no tube modeller
    /// (PresetDocument.swift:47).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tube: Option<TubeBlock>,
    #[serde(default)]
    pub channels: Vec<ChannelBlock>,
    #[serde(default)]
    pub matrix: Vec<CrosspointBlock>,
    #[serde(default)]
    pub io: IoBlock,
}

/// Whether a block holds no value at all.
fn says_nothing<T: Default + PartialEq>(block: &T) -> bool {
    *block == T::default()
}

/// Provenance. Informational only, but a mismatch is worth telling the user about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    #[serde(default)]
    pub wire_format_version: i32,
    #[serde(default)]
    pub input_channel_count: i32,
    #[serde(default)]
    pub output_channel_count: i32,
    /// The source device's `MASTER_VOLUME_MODE_*` (config.h:484-485), so an
    /// import can say why it left master volume alone. Additive in the
    /// Console (PresetDocument.swift:90-93).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_volume_mode: Option<i32>,
    /// The source device's `OUTPUT_CONFIG_MODE_*` (config.h:497-498).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_config_mode: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GlobalBlock {
    /// Per-input trim, indexed by wire input 0..7. Shorter than the document's
    /// inputs when the device would not report the next one, since a position
    /// here is an input: that input and the ones after it are left alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_preamps_db: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_volume_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_volume_db: Option<f32>,
    /// Wire InputSource value. A listening choice rather than wiring, so it
    /// travels with the audio settings and not with the I/O block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_source: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lg_sound_sync_enabled: Option<bool>,
    /// App-side PEQ pair linking. The firmware has no notion of it, but losing
    /// it on import would surprise anyone who set it up.
    pub input_pair_linked: Vec<bool>,
}

/// Loudness compensation (PresetDocument.swift:149-164). A missing key leaves
/// the device's value alone, so a Windows file written before the output mask
/// existed does not switch loudness off on every output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoudnessBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_spl: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity_pct: Option<f32>,
    /// Bit k: loudness runs on output k (bulk_params.h:64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mask: Option<i32>,
}

/// Crossfeed (PresetDocument.swift:166-185).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrossfeedBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub itd: Option<bool>,
    /// Bit p: crossfeed runs on output pair p (bulk_params.h:76).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_pair_mask: Option<i32>,
    /// The extension channel's `EXT_CROSSFEED_PAIR_MAP`, its four bytes
    /// (PresetDocument.swift:172-178; `dspi_core::files` and the Android app
    /// write the same key). Absent means the document does not say, which
    /// leaves the device's map alone. A value that is not an integer array
    /// reads as absent, as the Console reads it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_pair_map"
    )]
    pub pair_map: Option<Vec<i32>>,
}

/// `pairMap`, tolerating anything that is not a list of integers
/// (PresetDocument.swift:199).
fn lenient_pair_map<'de, D>(d: D) -> Result<Option<Vec<i32>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or(None))
}

impl CrossfeedBlock {
    /// The map to apply, as the Console picks it
    /// (`CrossfeedBlock.pairMap(pairCount:)`, PresetDocument.swift:185-189):
    /// the normalized map when valid for `pairs` pairs, otherwise the default
    /// (a file from a wider platform, or a hand edit). `None` when the document
    /// carries no map.
    pub fn pair_map_for(&self, pairs: u8) -> Option<PairMap> {
        let raw = self.pair_map.as_ref()?;
        if raw.len() > PairMap::SIZE {
            return Some(PairMap([0; PairMap::SIZE]));
        }
        let mut bytes = [0u8; PairMap::SIZE];
        for (b, v) in bytes.iter_mut().zip(raw) {
            *b = (*v).clamp(0, 255) as u8;
        }
        let map = PairMap(bytes);
        Some(if pair_map_valid(&map, pairs) {
            map.normalized()
        } else {
            PairMap([0; PairMap::SIZE])
        })
    }
}

/// The firmware's `crossfeed_map_valid`, as `isValid(pairCount:)`
/// (ExtensionChannel.swift:156-171).
fn pair_map_valid(map: &PairMap, pairs: u8) -> bool {
    if pairs == 0 || pairs as usize > PairMap::PAIRS {
        return false;
    }
    let mut used = 0u32;
    for p in 0..PairMap::PAIRS {
        if p >= pairs as usize {
            if map.0[p] != 0 {
                return false;
            }
            continue;
        }
        let (l, r) = map.pair(p);
        if l >= 2 * pairs || r >= 2 * pairs || l == r {
            return false;
        }
        let bits = (1u32 << l) | (1u32 << r);
        if used & bits != 0 {
            return false;
        }
        used |= bits;
    }
    true
}

/// The volume leveller (PresetDocument.swift:187-210).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LevellerBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookahead: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_db: Option<f32>,
    /// `[detector_mask, apply_mask]` on the wire (config.h:441).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector_mask: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_mask: Option<i32>,
}

/// Psychoacoustic bass (PresetDocument.swift:212-233).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PsybassBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harmonics_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mask: Option<i32>,
}

/// The stereo upmixer (PresetDocument.swift:311-346). The two modes are the raw
/// `UPMIX_CENTER_*` and `UPMIX_SURROUND_*` numbers (upmix.h:78-85).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpmixBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surround_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_width_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector_hpf_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surround_delay_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surround_hpf_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surround_lpf_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decorr_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_db: Option<f32>,
}

/// The subharmonic synthesizer, as the Console's `SubharmBlock`
/// (PresetDocument.swift:235-268). A key an older writer did not have, such as
/// the third band's, leaves the device's value alone. Solo is absent by design:
/// it is runtime only, and no saved configuration may switch the program off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubharmBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mask: Option<i32>,
    /// `SUBHARM_SELECT_*` (subharm.h:71-73), as the wire number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_mode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_depth_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_hold_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_pairs: Option<bool>,
}

/// The tube modeller, as the Console's `TubeBlock` (PresetDocument.swift:
/// 270-309). Every value is stored as the firmware holds it, the four
/// character values included, so a file saved on a tube type restores that
/// exact sound even if a later firmware retunes the row. Defaults are the
/// firmware's (tube.h:60-73).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TubeBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mask: Option<i32>,
    /// 0 Custom, 1..=`TUBE_TYPE_MAX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tube_type: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bias_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asym_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardness_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sag_pct: Option<f32>,
    /// 0..=`TUBE_RECT_MAX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rectifier: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xfmr_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xfmr_damping: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xfmr_res_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_db: Option<f32>,
}

/// One output's limiter, as the Console's `LimiterBlock`
/// (PresetDocument.swift:406-432). Defaults are the firmware's
/// (limiter.h:32-33).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LimiterBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_ms: Option<f32>,
    /// 0 unlinked, 1..=`LIMITER_LINK_GROUP_MAX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelBlock {
    /// The Windows Console's channel id, which is not the firmware's unified
    /// channel index: see [`channel_id`]. Written for every channel, and read
    /// when the entry carries no `inputIndex` or `outputIndex`.
    pub channel_id: i32,
    #[serde(default)]
    pub name: String,
    pub is_output: bool,
    /// The unified channel index (config.h:788-790): inputs from 0, outputs
    /// from `CH_OUT_1`. Written as the Console writes it
    /// (PresetDocumentTransfer.swift:192); neither Console places a channel
    /// by it, and nor does this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq_channel: Option<i32>,
    /// Wire input index 0..7, on inputs only. Wins over `channelId`
    /// (PresetDocument.swift:679-683).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_index: Option<i32>,
    /// Matrix output index, on outputs only. Wins over `channelId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_index: Option<i32>,
    /// Pre-matrix channel delay (`REQ_SET_DELAY`, config.h:245).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Post-matrix output delay (`REQ_SET_OUTPUT_DELAY`, config.h:311), a
    /// separate value from `delayMs` that the Windows schema omits. Outputs
    /// only; absent leaves the device's alone (PresetDocument.swift:372-374).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_delay_ms: Option<f32>,
    #[serde(default)]
    pub eq: Vec<BandBlock>,
    /// Empty for inputs.
    #[serde(default)]
    pub crossover: Vec<BandBlock>,
    /// This output's limiter (PresetDocument.swift:375-379). Absent on inputs
    /// and when the source device had no limiter, which leaves the device's
    /// own alone. Applied only with the hardware I/O option, because the
    /// firmware keeps it with the output configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limiter: Option<LimiterBlock>,
}

/// Where a channel entry lands on a device: a wire input index, or a matrix
/// output index (the Console's `PresetChannelRef`, PresetDocument.swift:
/// 617-621). Inputs sort before outputs, which is the order the Console
/// applies them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelRef {
    Input(u8),
    Output(u8),
}

impl ChannelRef {
    /// The unified channel index on a device with `num_inputs` inputs:
    /// outputs start at `CH_OUT_1 = NUM_INPUT_CHANNELS` (config.h:788-790).
    pub fn unified(self, num_inputs: u8) -> u8 {
        match self {
            ChannelRef::Input(i) => i,
            ChannelRef::Output(o) => num_inputs.saturating_add(o),
        }
    }
}

/// The Windows Console's channel ids, which both Consoles write as
/// `channelId` (PresetDocument.swift:623-672).
///
/// They were laid down when the device had two inputs, and outputs grew in
/// the middle: inputs 0 and 1 keep their index, outputs follow from 2, and
/// the extra inputs of the eight-input model sit above the widest output
/// bank at 11..16. On RP2040 (two inputs, five outputs, config.h:776-777)
/// this coincides with the unified index; on RP2350 (eight inputs, nine
/// outputs, config.h:773-774) it does not, which is why the unified index
/// must never be written here. The Console gives RP2350's PDM output id 10
/// by a special case, which is the same number the arithmetic gives, so no
/// platform test is needed and none is compiled in.
///
/// These numbers are the file format's, not the device's: a device's own
/// input and output counts decide only whether a resolved channel exists.
pub mod channel_id {
    use super::ChannelRef;

    /// Inputs below this keep their index as their id: the stereo inputs
    /// (`NUM_STEREO_INPUTS`, config.h:784; the Console's
    /// `BASE_MATRIX_INPUTS`, Constants.swift:772).
    const STEREO_INPUTS: i32 = 2;
    /// The id of the first output (S/PDIF 1 L).
    const OUTPUT_BASE: i32 = 2;
    /// The id of wire input 2, the first extra input.
    const EXTRA_INPUT_BASE: i32 = 11;
    /// Extra inputs the schema has ids for: eight inputs less the stereo two
    /// (`MAX_MATRIX_INPUTS`, Constants.swift:771).
    const EXTRA_INPUTS: i32 = 6;

    /// The id for a wire input index.
    pub fn for_input(input: u8) -> i32 {
        let i = input as i32;
        if i < STEREO_INPUTS {
            i
        } else {
            EXTRA_INPUT_BASE + (i - STEREO_INPUTS)
        }
    }

    /// The id for a matrix output index.
    pub fn for_output(output: u8) -> i32 {
        OUTPUT_BASE + output as i32
    }

    /// The channel an id names, before asking whether the device has it
    /// (`PresetChannelID.ref(forID:platform:)`). Inputs are tested first, so
    /// 11..16 are always inputs.
    pub fn resolve(id: i32) -> Option<ChannelRef> {
        if (0..STEREO_INPUTS).contains(&id) {
            return Some(ChannelRef::Input(id as u8));
        }
        if (EXTRA_INPUT_BASE..EXTRA_INPUT_BASE + EXTRA_INPUTS).contains(&id) {
            return Some(ChannelRef::Input(
                (STEREO_INPUTS + id - EXTRA_INPUT_BASE) as u8,
            ));
        }
        u8::try_from(id - OUTPUT_BASE).ok().map(ChannelRef::Output)
    }

    /// Whether an entry's direction agrees with the id table. Every file
    /// either Console writes agrees; an older Terminal file written on an
    /// eight-input device does not.
    pub(super) fn agrees(id: i32, is_output: bool) -> bool {
        match resolve(id) {
            Some(ChannelRef::Input(_)) => !is_output,
            Some(ChannelRef::Output(_)) => is_output,
            None => false,
        }
    }
}

impl ChannelBlock {
    /// The channel this entry refers to, as the Console reads it
    /// (PresetDocument.swift:674-684): its own `inputIndex` or `outputIndex`
    /// when present, otherwise the shared `channelId`.
    ///
    /// `legacy_inputs` is set only for a document an older Terminal wrote,
    /// whose `channelId` is the unified index on a device with that many
    /// inputs (see [`legacy_numbering`]).
    pub fn placement(&self, legacy_inputs: Option<u8>) -> Option<ChannelRef> {
        if self.is_output
            && let Some(o) = self.output_index
        {
            return u8::try_from(o).ok().map(ChannelRef::Output);
        }
        if !self.is_output
            && let Some(i) = self.input_index
        {
            return u8::try_from(i).ok().map(ChannelRef::Input);
        }
        if let Some(n) = legacy_inputs {
            let n = n as i32;
            return match self.channel_id {
                id if (0..n).contains(&id) => Some(ChannelRef::Input(id as u8)),
                id => u8::try_from(id - n).ok().map(ChannelRef::Output),
            };
        }
        channel_id::resolve(self.channel_id)
    }
}

/// Whether a document uses the numbering Terminal builds before this one
/// wrote, and with how many inputs.
///
/// Those builds wrote the unified channel index as `channelId` and none of
/// the Console's own index fields. On a two-input device the two numberings
/// coincide, so only an eight-input document can differ, and there the
/// difference shows: the older numbering marks ids 2..7 as inputs and 8..16
/// as outputs, where the id table has 2..10 as outputs and 11..16 as inputs.
/// A document counts as older only when no entry carries an index field,
/// every entry agrees with the unified numbering, and at least one disagrees
/// with the id table. No file either Console writes can pass that test,
/// since every one of their entries agrees with the table.
///
/// The input count is the document's own `inputChannelCount`, or the
/// connected device's when the document does not say.
pub fn legacy_numbering(doc: &PresetDocument, device_inputs: u8) -> Option<u8> {
    let n = u8::try_from(doc.meta.input_channel_count)
        .ok()
        .filter(|&n| n > 0)
        .unwrap_or(device_inputs);
    let own_fields = doc
        .channels
        .iter()
        .any(|c| c.eq_channel.is_some() || c.input_index.is_some() || c.output_index.is_some());
    let unified = doc
        .channels
        .iter()
        .all(|c| c.channel_id >= 0 && (c.channel_id < n as i32) != c.is_output);
    let table = doc
        .channels
        .iter()
        .all(|c| channel_id::agrees(c.channel_id, c.is_output));
    (!own_fields && unified && !table).then_some(n)
}

/// One filter band. Field meanings follow the wire encoding, including the
/// Linkwitz Transform's reuse of them: `freq` is f0, `q` is Q0, `gain` carries
/// fp in Hz, and `qp` is the target pole Q.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BandBlock {
    /// Raw wire `FilterType`, never a name.
    pub r#type: i32,
    pub freq_hz: f32,
    pub q: f32,
    pub gain: f32,
    pub qp: f32,
    pub bypass: bool,
}

impl Default for BandBlock {
    fn default() -> Self {
        Self {
            r#type: 0,
            freq_hz: 1000.0,
            q: 0.707,
            gain: 0.0,
            qp: 0.707,
            bypass: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrosspointBlock {
    pub input: i32,
    pub output: i32,
    pub enabled: bool,
    pub invert: bool,
    pub gain_db: f32,
}

/// Physical wiring. Applied only when the user opts in, since GPIO assignments
/// belong to a board rather than to a listening setup.
///
/// Every scalar is optional, which is not the reference implementation's shape
/// and is deliberate: there, an absent key falls back to a platform default and
/// is written anyway. Here an absent key means "the document does not say", and
/// nothing is written. The difference matters because a hand-edited or
/// truncated file would otherwise read as "move the bit clock to GPIO 0", and a
/// pin move is exactly the change that is expensive to undo. A file written by
/// either Console carries every key, so interchange is unaffected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IoBlock {
    /// GPIOs for the pin outputs: one per S/PDIF slot, then the PDM sub
    /// (config.h:667-673, `NUM_PIN_OUTPUTS`).
    #[serde(default)]
    pub output_pins: Vec<u8>,
    /// 0 = S/PDIF, 1 = I2S per output slot (config.h:615-617).
    #[serde(default)]
    pub output_slot_types: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_bck_pin: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mck_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mck_pin: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mck_multiplier: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_clock_mode: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_clock_pin_mode: Option<u8>,
    /// The slave clock pair, stored while dormant so entering SPLIT finds it
    /// already valid (config.h:485-487).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_bck_pin_slave: Option<u8>,
    /// S/PDIF RX GPIOs for inputs 0..2. The shared schema is three long; the
    /// fourth input is newer than the schema and travels in `spdifRxPin4`.
    #[serde(default)]
    pub spdif_rx_pins: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spdif_rx_pin4: Option<u8>,
    /// Enable mask for the optional inputs: bit 0 is input 1, bit 1 input 2,
    /// bit 2 input 3 (config.h:454, indices 1..3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spdif_enabled_ext: Option<u8>,
    #[serde(default)]
    pub i2s_rx_pins: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_input_channels: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2s_input_rate_hz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adat_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adat_pin: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adat_input_enabled: Option<bool>,
    /// 0xFF means unset, which is the ADAT input's own default
    /// (config.h:595-604).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adat_input_pin: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adat_input_clock_mode: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dac_hw_mute: Option<DacHwMuteBlock>,
}

/// The external DAC's mute pin (config.h:439-442, a 16-byte `DacHwMuteConfig`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DacHwMuteBlock {
    pub enabled: bool,
    pub active_low: bool,
    pub pin: u8,
    pub hold_ms: u16,
    pub release_ms: u16,
}

impl DacHwMuteBlock {
    /// The 16-byte wire layout: `{enabled, active_low, pin, rsvd, hold_ms u16,
    /// release_ms u16}`, the rest reserved and zero.
    pub fn encode(&self) -> Vec<u8> {
        let mut d = vec![0u8; 16];
        d[0] = self.enabled as u8;
        d[1] = self.active_low as u8;
        d[2] = self.pin;
        d[4..6].copy_from_slice(&self.hold_ms.to_le_bytes());
        d[6..8].copy_from_slice(&self.release_ms.to_le_bytes());
        d
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }
        Some(Self {
            enabled: bytes[0] != 0,
            active_low: bytes[1] != 0,
            pin: bytes[2],
            hold_ms: u16::from_le_bytes([bytes[4], bytes[5]]),
            release_ms: u16::from_le_bytes([bytes[6], bytes[7]]),
        })
    }
}

/// Which parts of a document an import should apply.
///
/// Only the audio processing is on by default. Volume levels would change how
/// loud a room gets, and hardware wiring describes a board; importing either
/// without being asked is the kind of surprise that loses trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOptions {
    pub audio_processing: bool,
    pub volume_levels: bool,
    pub hardware_io: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            audio_processing: true,
            volume_levels: false,
            hardware_io: false,
        }
    }
}

/// What an import actually did, so the user is told rather than left to infer it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApplyReport {
    pub channels_applied: usize,
    pub bands_applied: usize,
    pub crossover_bands_applied: usize,
    pub crosspoints_applied: usize,
    /// Channels the document carries that this device does not have.
    pub missing_channels: Vec<String>,
    /// Blocks skipped for lack of a feature, or because applying them would
    /// have conflicted.
    pub skipped: Vec<String>,
    /// Settings the document holds no value for, which were left as they are.
    pub left_alone: Vec<String>,
    /// What the user should know about the document itself.
    pub warnings: Vec<String>,
}

impl ApplyReport {
    /// Record a reason once.
    ///
    /// Per-band and per-pin problems would otherwise repeat for every item in
    /// the file and bury the one line that matters.
    pub fn skip(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.skipped.contains(&reason) {
            self.skipped.push(reason);
        }
    }

    /// Nothing was missing, nothing was refused and nothing was left out.
    pub fn is_clean(&self) -> bool {
        self.missing_channels.is_empty() && self.skipped.is_empty() && self.left_alone.is_empty()
    }

    /// The result report, worded as the Console words it.
    ///
    /// The closing line is the one that stops someone power-cycling and losing
    /// the whole import, so it is not dropped when everything went well. A dry
    /// run omits it, because nothing is live either.
    pub fn lines(&self, dry_run: bool) -> Vec<String> {
        let mut lines = vec![format!(
            "{} {} channels, {} EQ bands, {} crossover bands, {} crosspoints.",
            if dry_run { "Would apply" } else { "Applied" },
            self.channels_applied,
            self.bands_applied,
            self.crossover_bands_applied,
            self.crosspoints_applied
        )];

        if !self.missing_channels.is_empty() {
            lines.push(format!(
                "Not present on this device: {}",
                self.missing_channels.join(", ")
            ));
        }
        for skipped in &self.skipped {
            lines.push(format!("Skipped: {skipped}"));
        }
        if !self.left_alone.is_empty() {
            lines.push(format!(
                "Left as they are, because the file holds no value for them: {}",
                self.left_alone.join(", ")
            ));
        }
        for warning in &self.warnings {
            lines.push(format!("Warning: {warning}"));
        }

        if !dry_run {
            lines.push(
                "These changes are live but not yet stored on the device. \
                 Save them to a preset slot to keep them."
                    .into(),
            );
        }
        lines
    }
}

/// Parse a document, refusing anything this build cannot honestly interpret.
pub fn parse(text: &str) -> Result<PresetDocument, PresetFileError> {
    if text.trim().is_empty() {
        return Err(PresetFileError::Empty);
    }
    let text = text.trim_start_matches('\u{feff}');

    let doc: PresetDocument =
        serde_json::from_str(text).map_err(|e| PresetFileError::Invalid(e.to_string()))?;

    if doc.schema_version <= 0 {
        return Err(PresetFileError::Invalid(
            "missing or invalid schemaVersion".into(),
        ));
    }
    // Refuse a document from the future rather than guessing at blocks we do
    // not understand and applying a partial tuning.
    if doc.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(PresetFileError::FromTheFuture {
            got: doc.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if doc.channels.is_empty() {
        return Err(PresetFileError::NoChannels);
    }
    Ok(doc)
}

pub fn write(doc: &PresetDocument) -> String {
    serde_json::to_string_pretty(doc).unwrap_or_default()
}

/// Write a document's text to a file, whole or not at all: beside the target,
/// then renamed over it, so a full disk or a crash leaves the old file as it
/// was. A symbolic link is followed and an existing file keeps its permissions.
/// Something that is not a regular file (a pipe, `/dev/stdout`) is written
/// directly, since there is nothing there to replace.
pub fn save_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let existing = std::fs::metadata(path).ok();
    if existing.as_ref().is_some_and(|m| !m.is_file()) {
        return std::fs::write(path, text);
    }
    let target = match &existing {
        Some(_) => std::fs::canonicalize(path)?,
        None => path.to_path_buf(),
    };
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = target.with_file_name(name);
    let written = std::fs::File::create(&temp).and_then(|mut f| {
        f.write_all(text.as_bytes())?;
        if let Some(m) = &existing {
            f.set_permissions(m.permissions())?;
        }
        f.sync_all()
    });
    let renamed = written.and_then(|()| std::fs::rename(&temp, &target));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    renamed
}

/// A document's channels placed on a device, and the names of those it has
/// nowhere to put.
pub type Placement<'a> = std::collections::BTreeMap<ChannelRef, &'a ChannelBlock>;

/// Place a document's channels on a device with this many inputs and
/// outputs, as the Console does (PresetDocumentTransfer.swift:302-313).
///
/// Each entry lands where its own index fields or its `channelId` say, read
/// against the connected device; one that names a channel the device does
/// not have is reported rather than moved somewhere else, so an eight-input
/// document on a two-input part reports its extra inputs. A duplicated
/// channel takes the last entry rather than failing.
pub fn place_channels(
    doc: &PresetDocument,
    num_inputs: u8,
    num_outputs: u8,
) -> (Placement<'_>, Vec<String>) {
    let legacy = legacy_numbering(doc, num_inputs);
    let mut placed = Placement::new();
    let mut missing = Vec::new();

    for c in &doc.channels {
        let exists = |r: &ChannelRef| match *r {
            ChannelRef::Input(i) => i < num_inputs,
            ChannelRef::Output(o) => o < num_outputs,
        };
        match c.placement(legacy).filter(exists) {
            Some(r) => {
                placed.insert(r, c);
            }
            None => missing.push(if c.name.is_empty() {
                format!("channel {}", c.channel_id)
            } else {
                c.name.clone()
            }),
        }
    }
    (placed, missing)
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

use crate::cs_model::{ApplyError, CsData, apply as cs_apply};
use crate::state::DeviceState;
use crate::surfaces::{
    CsBinding, CsDisplayCfg, CsDisplayPage, CsGroup, CsMacro, CsMacroHeaderWire, CsMacroStep,
    CsStatusPacket, GPIO_UNUSED, IrCommand, Limits, Surfaces, explain_status,
};
use crate::{EnableOutcome, Outcome, Session};
use dspi_proto::ext::PairMap;
use dspi_proto::generated::status;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::value::{EqParamPacket, Value};
use dspi_proto::wire_compat::{self, feat};
use dspi_transport::TransportError;

/// `MASTER_VOLUME_MODE_WITH_PRESET` (config.h:485), which the generated
/// constants do not carry.
const MASTER_VOLUME_MODE_WITH_PRESET: u8 = 1;

/// Crossover bands per output, at wire band indices 20..23.
const CROSSOVER_BANDS: u8 = 4;
const CROSSOVER_FIRST_BAND: u8 = 20;

/// Apply a document to a device.
///
/// Everything goes through the ordinary write path, so each value gets the same
/// clamping, capability gating and readback verification as a typed command.
/// Nothing is applied that the options did not ask for, and the report says what
/// actually happened rather than leaving the user to infer it from the UI.
///
/// The input pair links (`global.inputPairLinked`) are the app's, not the
/// device's, so this leaves them to the caller; the Terminal applies them to
/// its own link state after this returns.
pub fn apply(session: &mut Session, doc: &PresetDocument, options: ApplyOptions) -> ApplyReport {
    let mut report = ApplyReport::default();

    let caps = session.capabilities().clone();
    let (placed, missing) = place_channels(doc, caps.num_inputs, caps.num_outputs);
    report.missing_channels = missing;
    report.left_alone = omitted(doc, options);
    report.warnings.extend(old_format_warning(doc));

    if !options.audio_processing {
        report
            .skipped
            .push("audio processing (not requested)".into());
    } else {
        apply_output_enables(session, &placed, &mut report);

        // One channel at a time, inputs first, as the Console does
        // (PresetDocumentTransfer.swift:394-415).
        for (&r, block) in &placed {
            apply_channel(session, r, block, &mut report);
            if let ChannelRef::Input(i) = r
                && let Some(db) = doc.global.input_preamps_db.get(i as usize)
            {
                let _ = session.write("pre", &[i], Value::Float(*db));
            }
        }

        for c in &doc.matrix {
            if c.input as u8 >= caps.num_inputs || c.output as u8 >= caps.num_outputs {
                continue;
            }
            // The crosspoint packet carries every field at once.
            let mut payload = vec![
                c.input as u8,
                c.output as u8,
                c.enabled as u8,
                c.invert as u8,
            ];
            payload.extend_from_slice(&c.gain_db.to_le_bytes());
            if session
                .write(
                    "mix",
                    &[c.input as u8, c.output as u8],
                    Value::Bytes(payload),
                )
                .is_ok()
            {
                report.crosspoints_applied += 1;
            }
        }

        apply_features(session, doc, &mut report);
    }

    if options.volume_levels {
        apply_volumes(session, doc, &mut report);
    } else {
        report.skip("volume levels (not requested)");
    }

    if options.hardware_io {
        apply_io(session, &doc.io, &mut report);
        apply_limiters(session, &placed, &mut report);
    } else {
        report.skip("hardware I/O (not requested)");
    }

    // The input source picks which wiring the device listens to, so it goes
    // after the wiring has moved rather than before it.
    if options.audio_processing
        && let Some(source) = doc.global.input_source
    {
        let v = Value::Choice(source);
        if let Err(crate::WriteError::Unavailable { .. }) = session.write("in.source", &[], v) {
            report.skip("Input source (not supported by this firmware)");
        }
    }

    report
}

/// The warning a document from older firmware deserves. Exports made from such
/// firmware before TM21 hold 0, false or a default for whatever it would not
/// report, and nothing in the file tells those from settings. A document that
/// does not name its format gets none.
pub fn old_format_warning(doc: &PresetDocument) -> Option<String> {
    let wire = doc.meta.wire_format_version;
    (wire > 0 && wire < i32::from(WIRE_FORMAT_VERSION)).then(|| {
        format!(
            "this file comes from older firmware (wire V{wire}; this build speaks \
             V{WIRE_FORMAT_VERSION}). An export made from that firmware by an older build can \
             hold 0 dB or off for settings the firmware did not report, so check the levels."
        )
    })
}

/// Every setting a document holds no value for, as its JSON key. A block with
/// nothing in it is named once. For a feature block the document does not have
/// at all, the source had no such feature, so that is not listed.
pub fn omitted_settings(doc: &PresetDocument) -> Vec<String> {
    omitted(
        doc,
        ApplyOptions {
            audio_processing: true,
            volume_levels: true,
            hardware_io: true,
        },
    )
}

/// Names the listed fields that are `None`, or the whole block if all of them are.
macro_rules! omitted_keys {
    ($out:expr, $block:expr, $name:literal, $($field:ident => $key:literal),+ $(,)?) => {
        if says_nothing($block) {
            $out.push($name.to_string());
        } else {
            $(if $block.$field.is_none() {
                $out.push(concat!($name, ".", $key).to_string());
            })+
        }
    };
}

/// [`omitted_settings`], for the parts of a document these options apply. The
/// `io` block is not listed: its keys depend on the source board's features, so
/// an absent one says nothing, and [`apply_io`] always left those alone.
fn omitted(doc: &PresetDocument, options: ApplyOptions) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let g = &doc.global;
    if options.audio_processing {
        let inputs = doc.channels.iter().filter(|c| !c.is_output).count();
        if g.input_preamps_db.len() < inputs {
            out.push("global.inputPreampsDb".into());
        }
        for (missing, key) in [
            (g.bypass.is_none(), "global.bypass"),
            (g.input_source.is_none(), "global.inputSource"),
            (
                g.lg_sound_sync_enabled.is_none(),
                "global.lgSoundSyncEnabled",
            ),
        ] {
            if missing {
                out.push(key.into());
            }
        }
    }
    if options.volume_levels {
        for (missing, key) in [
            (g.master_volume_db.is_none(), "global.masterVolumeDb"),
            (g.user_volume_db.is_none(), "global.userVolumeDb"),
        ] {
            if missing {
                out.push(key.into());
            }
        }
    }
    for c in &doc.channels {
        let mut keys: Vec<(bool, &str)> = Vec::new();
        if options.audio_processing {
            keys.push((c.delay_ms.is_none(), "channels.delayMs"));
            keys.push((c.eq.is_empty(), "channels.eq"));
            if c.is_output {
                keys.push((c.gain_db.is_none(), "channels.gainDb"));
                keys.push((c.muted.is_none(), "channels.muted"));
                keys.push((c.enabled.is_none(), "channels.enabled"));
                keys.push((c.output_delay_ms.is_none(), "channels.outputDelayMs"));
                keys.push((c.crossover.is_empty(), "channels.crossover"));
            }
        }
        if let (true, Some(l)) = (options.hardware_io, &c.limiter) {
            keys.push((l.enabled.is_none(), "channels.limiter.enabled"));
            keys.push((l.threshold_db.is_none(), "channels.limiter.thresholdDb"));
            keys.push((l.release_ms.is_none(), "channels.limiter.releaseMs"));
            keys.push((l.link_group.is_none(), "channels.limiter.linkGroup"));
        }
        for (missing, key) in keys {
            if missing && !out.iter().any(|k| k == key) {
                out.push(key.into());
            }
        }
    }
    if !options.audio_processing {
        return out;
    }
    if doc.matrix.is_empty() {
        out.push("matrix".into());
    }
    omitted_keys!(out, &doc.loudness, "loudness",
        enabled => "enabled", ref_spl => "refSpl",
        intensity_pct => "intensityPct", output_mask => "outputMask");
    omitted_keys!(out, &doc.crossfeed, "crossfeed",
        enabled => "enabled", preset => "preset", freq_hz => "freqHz", feed_db => "feedDb",
        itd => "itd", output_pair_mask => "outputPairMask");
    omitted_keys!(out, &doc.leveller, "leveller",
        enabled => "enabled", speed => "speed", lookahead => "lookahead",
        amount_pct => "amountPct", max_gain_db => "maxGainDb", gate_db => "gateDb",
        detector_mask => "detectorMask", apply_mask => "applyMask");
    if let Some(b) = &doc.psybass {
        omitted_keys!(out, b, "psybass",
            enabled => "enabled", cutoff_hz => "cutoffHz", harmonics_db => "harmonicsDb",
            drive_db => "driveDb", character_pct => "characterPct",
            original_db => "originalDb", output_mask => "outputMask");
    }
    if let Some(b) = &doc.upmix {
        omitted_keys!(out, b, "upmix",
            enabled => "enabled", center_mode => "centerMode", surround_mode => "surroundMode",
            strength_pct => "strengthPct", center_width_pct => "centerWidthPct",
            threshold_pct => "thresholdPct", attack_ms => "attackMs", release_ms => "releaseMs",
            detector_hpf_hz => "detectorHpfHz", surround_delay_ms => "surroundDelayMs",
            surround_hpf_hz => "surroundHpfHz", surround_lpf_hz => "surroundLpfHz",
            decorr_pct => "decorrPct", presence_db => "presenceDb");
    }
    if let Some(b) = &doc.subharm {
        omitted_keys!(out, b, "subharm",
            enabled => "enabled", low_db => "lowDb", high_db => "highDb", top_db => "topDb",
            boost_db => "boostDb", output_mask => "outputMask", select_mode => "selectMode",
            select_depth_pct => "selectDepthPct", select_hold_ms => "selectHoldMs",
            ceiling_db => "ceilingDb", link_pairs => "linkPairs");
    }
    if let Some(b) = &doc.tube {
        omitted_keys!(out, b, "tube",
            enabled => "enabled", output_mask => "outputMask", tube_type => "tubeType",
            drive_db => "driveDb", bias_pct => "biasPct", asym_db => "asymDb",
            hardness_pct => "hardnessPct", sag_pct => "sagPct", rectifier => "rectifier",
            xfmr_enabled => "xfmrEnabled", xfmr_damping => "xfmrDamping",
            xfmr_res_hz => "xfmrResHz", mix_pct => "mixPct", trim_db => "trimDb");
    }
    out
}

/// Output enables: every disable, then every enable
/// (PresetDocumentTransfer.swift:377-392). An enable can collide with an
/// output the document is about to switch off (PDM against the Core 1 EQ
/// workers), so freeing first is what lets the pair land. An enable that
/// would still collide is reported in the Console's words, not forced.
fn apply_output_enables(session: &mut Session, placed: &Placement, report: &mut ApplyReport) {
    for enabling in [false, true] {
        for (&r, block) in placed {
            let ChannelRef::Output(o) = r else { continue };
            if block.enabled != Some(enabling) {
                continue;
            }
            let now = session
                .read("out.enable", &[o])
                .ok()
                .and_then(|v| v.as_bool());
            if now == Some(enabling) {
                continue;
            }
            if let Ok(EnableOutcome::NeedsConfirm(_)) = session.enable_output(o, enabling) {
                report.skip(format!(
                    "{} could not be enabled (conflicts with another output)",
                    block.name
                ));
            }
        }
    }
}

/// One channel's name, delay, output strip and filter banks
/// (PresetDocumentTransfer.swift:451-505).
fn apply_channel(
    session: &mut Session,
    r: ChannelRef,
    block: &ChannelBlock,
    report: &mut ApplyReport,
) {
    let caps = session.capabilities().clone();
    let ch = r.unified(caps.num_inputs);

    let current_name = caps
        .channels
        .iter()
        .find(|c| c.index == ch)
        .map(|c| c.name.as_str());
    if !block.name.is_empty() && current_name != Some(block.name.as_str()) {
        let _ = session.write("ch.name", &[ch], Value::Text(block.name.clone()));
    }
    if let Some(ms) = block.delay_ms {
        let _ = session.write("ch.delay", &[ch], Value::Float(ms));
    }

    if let ChannelRef::Output(o) = r {
        if let Some(db) = block.gain_db {
            let _ = session.write("out.gain", &[o], Value::Float(db));
        }
        if let Some(want) = block.muted {
            let muted = session
                .read("out.mute", &[o])
                .ok()
                .and_then(|v| v.as_bool());
            if muted != Some(want) {
                let _ = session.write("out.mute", &[o], Value::Bool(want));
            }
        }
        // Additive: a document without it leaves the output delay alone.
        if let Some(ms) = block.output_delay_ms {
            let _ = session.write("out.delay", &[o], Value::Float(ms));
        }
    }

    // A document with no bands for a channel leaves its EQ alone; one with
    // some flattens the rest, so an imported channel is never a blend of two
    // configurations. Bands past this device's bank are dropped.
    let flat = BandBlock::default();
    if !block.eq.is_empty() {
        for band in 0..caps.max_bands {
            let b = block.eq.get(band as usize).unwrap_or(&flat);
            if apply_band(session, ch, band, b).is_some() {
                report.bands_applied += 1;
            }
        }
    }
    if matches!(r, ChannelRef::Output(_)) && !block.crossover.is_empty() {
        for band in 0..CROSSOVER_BANDS {
            let b = block.crossover.get(band as usize).unwrap_or(&flat);
            if apply_band(session, ch, CROSSOVER_FIRST_BAND + band, b).is_some() {
                report.crossover_bands_applied += 1;
            }
        }
    }

    report.channels_applied += 1;
}

fn apply_band(session: &mut Session, channel: u8, band: u8, b: &BandBlock) -> Option<()> {
    let packet = EqParamPacket {
        channel,
        band,
        filter_type: dspi_proto::FilterType::from_raw(b.r#type as u8),
        bypass: b.bypass,
        freq: b.freq_hz,
        q: b.q,
        gain_db: b.gain,
        qp: Some(b.qp),
    };
    match session.write_band(&packet) {
        Ok(Outcome::Rejected { .. }) | Err(_) => None,
        Ok(_) => Some(()),
    }
}

/// Master and listening volume (PresetDocumentTransfer.swift:669-680).
/// Master volume belongs to a preset only when the device says so; in
/// independent mode it is device-global, and a file overwriting it would
/// fight the user's own setting.
fn apply_volumes(session: &mut Session, doc: &PresetDocument, report: &mut ApplyReport) {
    let mode = current_u8(session, "vol.master.mode", &[]);
    if let Some(db) = doc.global.master_volume_db {
        if mode == Some(MASTER_VOLUME_MODE_WITH_PRESET) {
            let _ = session.write("vol.master", &[], Value::Float(db));
        } else {
            report.skip("Master volume (device is in independent master-volume mode)");
        }
    }
    if let Some(db) = doc.global.user_volume_db {
        let _ = session.write("vol.user", &[], Value::Float(db));
    }
}

/// The feature blocks, in the Console's order
/// (PresetDocumentTransfer.swift:537-667): each feature's parameters before
/// its switch, so the device never runs it for a moment on the old values,
/// and each mask only where the firmware has one. A block the device lacks
/// is skipped with a reason.
fn apply_features(session: &mut Session, doc: &PresetDocument, report: &mut ApplyReport) {
    let caps = session.capabilities().clone();
    let has = |name: &str| caps.features.iter().any(|f| f.name == name && f.present);
    // Ahead of the crossfeed switch below, as the Console orders it.
    apply_pair_map(session, doc, report);
    // A key the document does not hold is left alone.
    let mut set = |path: &str, v: Option<Value>, label: &str, report: &mut ApplyReport| {
        let Some(v) = v else { return };
        if let Err(e) = session.write(path, &[], v) {
            // An absent feature is information, not a failure: a document from a
            // better-equipped device should still apply everything else.
            if matches!(e, crate::WriteError::Unavailable { .. }) {
                report.skip(format!("{label} (not on this device)"));
            }
        }
    };
    let mask = |m: Option<i32>| m.map(|m| Value::Mask(m as u16 as u32));
    let choice = |c: Option<i32>| c.map(|c| Value::Choice(c.clamp(0, 255) as u8));
    let float = |x: Option<f32>| x.map(Value::Float);
    let flag = |x: Option<bool>| x.map(Value::Bool);

    // The input source is applied at the very end of `apply`, after any
    // hardware wiring has moved.
    set("bypass", flag(doc.global.bypass), "EQ bypass", report);

    if has("lg_sound_sync") {
        set(
            "in.lg",
            flag(doc.global.lg_sound_sync_enabled),
            "LG Sound Sync",
            report,
        );
    } else if doc.global.lg_sound_sync_enabled == Some(true) {
        report.skip("LG Sound Sync (not supported by this firmware)");
    }

    let l = &doc.loudness;
    set("loud.ref", float(l.ref_spl), "loudness", report);
    set("loud.intensity", float(l.intensity_pct), "loudness", report);
    if has("loudness_output_mask") {
        set("loud.mask", mask(l.output_mask), "loudness", report);
    }
    set("loud.on", flag(l.enabled), "loudness", report);

    let c = &doc.crossfeed;
    set("cf.preset", choice(c.preset), "crossfeed", report);
    set("cf.freq", float(c.freq_hz), "crossfeed", report);
    set("cf.feed", float(c.feed_db), "crossfeed", report);
    set("cf.itd", flag(c.itd), "crossfeed", report);
    if has("crossfeed_output_mask") {
        set(
            "cf.outputs",
            mask(c.output_pair_mask.map(|m| m & 0xFF)),
            "crossfeed",
            report,
        );
    }
    set("cf.on", flag(c.enabled), "crossfeed", report);

    let v = &doc.leveller;
    set("lev.speed", choice(v.speed), "leveller", report);
    set("lev.lookahead", flag(v.lookahead), "leveller", report);
    set("lev.amount", float(v.amount_pct), "leveller", report);
    set("lev.maxgain", float(v.max_gain_db), "leveller", report);
    set("lev.gate", float(v.gate_db), "leveller", report);
    if has("leveller_masks") {
        // One two-byte parameter, detector first (config.h:441).
        // Half of it says nothing about the other half, so it needs both.
        let word = v
            .detector_mask
            .zip(v.apply_mask)
            .map(|(detector, apply)| (detector & 0xFF) | ((apply & 0xFF) << 8));
        set("lev.masks", mask(word), "leveller", report);
    }
    set("lev.on", flag(v.enabled), "leveller", report);

    if let Some(b) = &doc.psybass {
        if has("psychoacoustic_bass") {
            let label = "psychoacoustic bass";
            set("bass.cutoff", float(b.cutoff_hz), label, report);
            set("bass.harmonics", float(b.harmonics_db), label, report);
            set("bass.drive", float(b.drive_db), label, report);
            set("bass.character", float(b.character_pct), label, report);
            set("bass.original", float(b.original_db), label, report);
            set("bass.mask", mask(b.output_mask), label, report);
            set("bass.on", flag(b.enabled), label, report);
        } else {
            report.skip("Psychoacoustic bass (not supported by this firmware)");
        }
    }

    apply_subharm_and_tube(session, doc, report);

    if let Some(u) = &doc.upmix {
        if has("upmixer") {
            // UPMIX_PARAM_* (upmix.h:188-201) through the registry's rows.
            let mut set = |path: &str, v: Option<Value>| {
                if let Some(v) = v {
                    let _ = session.write(path, &[], v);
                }
            };
            set("up.center_mode", choice(u.center_mode));
            set("up.surround_mode", choice(u.surround_mode));
            set("up.strength", float(u.strength_pct));
            set("up.width", float(u.center_width_pct));
            set("up.threshold", float(u.threshold_pct));
            set("up.attack", float(u.attack_ms));
            set("up.release", float(u.release_ms));
            set("up.det_hpf", float(u.detector_hpf_hz));
            set("up.sur_delay", float(u.surround_delay_ms));
            set("up.sur_hpf", float(u.surround_hpf_hz));
            set("up.sur_lpf", float(u.surround_lpf_hz));
            set("up.decorr", float(u.decorr_pct));
            set("up.presence", float(u.presence_db));
            set("up.on", flag(u.enabled));
        } else {
            report.skip("Stereo upmixer (not supported by this device)");
        }
    }
}

/// The subharmonic synthesizer and the tube modeller, in the Console's order
/// (PresetDocumentTransfer.swift:600-644): every parameter first and the
/// switch last, so a block never plays half-applied, and the tube type
/// before the four values it loads. A stored value that differs from the
/// type's row then drops the type to Custom on the device (tube.c:122-212),
/// so a file matching its row keeps its type and one that does not still
/// restores its exact sound.
fn apply_subharm_and_tube(session: &mut Session, doc: &PresetDocument, report: &mut ApplyReport) {
    let caps = session.capabilities().clone();
    let has = |name: &str| caps.features.iter().any(|f| f.name == name && f.present);
    // A refused value is left to the re-read to show; the Console does not
    // report one either.
    let mut set = |path: &str, v: Option<Value>| {
        if let Some(v) = v {
            let _ = session.write(path, &[], v);
        }
    };
    let float = |x: Option<f32>| x.map(Value::Float);
    let flag = |x: Option<bool>| x.map(Value::Bool);
    let mask = |m: Option<i32>| m.map(|m| Value::Mask(m as u16 as u32));
    let choice = |c: Option<i32>| c.map(|c| Value::Choice(c.clamp(0, 255) as u8));

    if let Some(b) = &doc.subharm {
        if has("subharmonic_synth") {
            set("sub.low", float(b.low_db));
            set("sub.high", float(b.high_db));
            set("sub.boost", float(b.boost_db));
            set("sub.mask", mask(b.output_mask));
            set("sub.top", float(b.top_db));
            set("sub.select", choice(b.select_mode));
            set("sub.depth", float(b.select_depth_pct));
            set("sub.hold", float(b.select_hold_ms));
            set("sub.ceiling", float(b.ceiling_db));
            set("sub.link", flag(b.link_pairs));
            set("sub.on", flag(b.enabled));
        } else {
            report.skip("Subharmonic synthesizer (not supported by this firmware)");
        }
    }

    if let Some(b) = &doc.tube {
        if has("tube_preamp") {
            set("tube.type", b.tube_type.map(|t| Value::Int(t as i64)));
            set("tube.bias", float(b.bias_pct));
            set("tube.asym", float(b.asym_db));
            set("tube.hardness", float(b.hardness_pct));
            set("tube.sag", float(b.sag_pct));
            set("tube.drive", float(b.drive_db));
            set("tube.rectifier", choice(b.rectifier));
            set("tube.damping", float(b.xfmr_damping));
            set("tube.resonance", float(b.xfmr_res_hz));
            set("tube.xfmr", flag(b.xfmr_enabled));
            set("tube.mix", float(b.mix_pct));
            set("tube.trim", float(b.trim_db));
            set("tube.mask", mask(b.output_mask));
            set("tube.on", flag(b.enabled));
        } else {
            report.skip("Tube preamp (not supported by this firmware)");
        }
    }
}

/// The output limiters, from each output's block, in the Console's order
/// (`applyLimiterSettings`, Commands.swift:1658-1672).
///
/// They travel with the hardware I/O option, not with the audio, because
/// the firmware keeps them with the output configuration
/// (PresetDocumentTransfer.swift:422-445). A write to one member of a link
/// group moves the whole group (limiter.c:143-194), so every output about to
/// change is unlinked first, then given its values, and the groups are set
/// again last in ascending order, each joining output adopting the settings
/// its lowest member already holds. An output with no block is left alone.
fn apply_limiters(session: &mut Session, placed: &Placement, report: &mut ApplyReport) {
    let caps = session.capabilities().clone();
    // Placed as every other channel value is, so a limiter lands on the
    // output its gain and EQ land on.
    let blocks: std::collections::BTreeMap<u8, &LimiterBlock> = placed
        .iter()
        .filter_map(|(r, c)| match (r, &c.limiter) {
            (ChannelRef::Output(o), Some(l)) => Some((*o, l)),
            _ => None,
        })
        .collect();
    if blocks.is_empty() {
        return;
    }
    if !caps
        .features
        .iter()
        .any(|f| f.name == "output_limiter" && f.present)
    {
        report.skip("Output limiter (not supported by this firmware)");
        return;
    }
    for (&o, l) in &blocks {
        // A block silent on its group keeps it, and so its members' settings.
        if l.link_group.is_some() && current_u8(session, "limit.link", &[o]).is_some_and(|g| g != 0)
        {
            let _ = session.write("limit.link", &[o], Value::Int(0));
        }
    }
    for (&o, l) in &blocks {
        if let Some(db) = l.threshold_db {
            let _ = session.write("limit.threshold", &[o], Value::Float(db));
        }
        if let Some(ms) = l.release_ms {
            let _ = session.write("limit.release", &[o], Value::Float(ms));
        }
        if let Some(on) = l.enabled {
            let _ = session.write("limit.on", &[o], Value::Bool(on));
        }
    }
    for (&o, l) in &blocks {
        if let Some(group) = l.link_group.filter(|g| *g != 0) {
            let _ = session.write("limit.link", &[o], Value::Int(group as i64));
        }
    }
}

// ---------------------------------------------------------------------------
// Hardware I/O
// ---------------------------------------------------------------------------

/// Push the wiring block, in the reference implementation's order.
///
/// The order is the whole of the difficulty, because most of these settings
/// validate against each other. It follows `PresetDocumentTransfer.swift`
/// (`applyIO`, lines 571-715), with one deliberate departure noted at the MCK
/// step. Every write goes through [`Session::write`], so each one is validated,
/// gated and read back like a typed command; a `PIN_CONFIG_*` refusal is
/// recorded and the apply carries on with the next item, because a board that
/// cannot take one pin can usually still take the other twenty.
fn apply_io(session: &mut Session, io: &IoBlock, report: &mut ApplyReport) {
    let caps = session.capabilities().clone();
    let has = |name: &str| caps.features.iter().any(|f| f.name == name && f.present);

    // One pin output per S/PDIF slot plus the PDM sub (config.h:667-673), and a
    // slot is a stereo pair of the output channels.
    let slots = (caps.num_outputs.saturating_sub(1) / 2) as usize;
    let pdm_pin_index = slots as u8;
    let pdm_output = caps.num_outputs.saturating_sub(1);

    // 1. Slot types before pins (swift:584-588). A type change is deferred and
    //    reallocates the slot, so a pin written first would be reassigned.
    for (slot, want) in io.output_slot_types.iter().take(slots).enumerate() {
        if current_u8(session, "out.type", &[slot as u8]) == Some(*want) {
            continue;
        }
        attempt(
            session,
            report,
            &format!("Output {} type", slot + 1),
            "out.type",
            &[slot as u8],
            Value::Choice(*want),
        );
    }

    // 2. Output pins, the slots and then PDM (swift:591-614).
    for index in 0..=pdm_pin_index {
        let Some(want) = io.output_pins.get(index as usize).copied() else {
            continue;
        };
        if current_u8(session, "out.pin", &[index]) == Some(want) {
            continue;
        }
        let what = format!("Output {} GPIO", index + 1);
        attempt(
            session,
            report,
            &what,
            "out.pin",
            &[index],
            Value::Int(want as i64),
        );

        // The firmware refuses to move the PDM pin while PDM is running
        // (survey-firmware 6.7, OUTPUT_ACTIVE). Cycle the output the way the
        // Console does and put its enable state back afterwards.
        if index == pdm_pin_index
            && session.last_write_status() == Some(status::PIN_CONFIG_OUTPUT_ACTIVE as u8)
        {
            let was_enabled = session
                .read("out.enable", &[pdm_output])
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let _ = session.enable_output(pdm_output, false);
            report.skipped.retain(|s| !s.starts_with(&what));
            attempt(
                session,
                report,
                &what,
                "out.pin",
                &[index],
                Value::Int(want as i64),
            );
            let _ = session.enable_output(pdm_output, was_enabled);
        }
    }

    // 3. The I2S bit clock (swift:617); the word clock sits on the next pin up.
    if let Some(want) = io.i2s_bck_pin
        && current_u8(session, "i2s.bck", &[]) != Some(want)
    {
        attempt(
            session,
            report,
            "I2S BCK pin",
            "i2s.bck",
            &[],
            Value::Int(want as i64),
        );
    }

    // 4. The master clock (swift:618-622). Departure from the reference: the
    //    firmware answers OUTPUT_ACTIVE for a pin change while MCK is running
    //    (survey-firmware 6.7), so the enable is dropped first and the
    //    document's enable state applied last, rather than enable-then-pin.
    let mck_pin_moves = io
        .mck_pin
        .is_some_and(|want| current_u8(session, "i2s.mck.pin", &[]) != Some(want));
    let mck_live = current_bool(session, "i2s.mck");
    if mck_pin_moves && mck_live == Some(true) {
        attempt(
            session,
            report,
            "MCK enable",
            "i2s.mck",
            &[],
            Value::Bool(false),
        );
    }
    if let Some(want) = io.mck_pin
        && mck_pin_moves
    {
        attempt(
            session,
            report,
            "MCK pin",
            "i2s.mck.pin",
            &[],
            Value::Int(want as i64),
        );
    }
    // The document carries the multiple itself (128 or 256, matching the
    // reference implementation's schema); the wire carries a selector
    // (config.h:342-343, 0 = 128x, 1 = 256x).
    if let Some(want) = io.mck_multiplier.map(mck_selector)
        && current_u8(session, "i2s.mck.mult", &[]) != Some(want)
    {
        attempt(
            session,
            report,
            "MCK multiplier",
            "i2s.mck.mult",
            &[],
            Value::Choice(want),
        );
    }
    if let Some(want) = io.mck_enabled
        && current_bool(session, "i2s.mck") != Some(want)
    {
        attempt(
            session,
            report,
            "MCK enable",
            "i2s.mck",
            &[],
            Value::Bool(want),
        );
    }

    // 5. S/PDIF inputs (swift:625-649). Drop an enable before repinning so a
    //    move cannot clash with the pin it is leaving, and apply each pin
    //    before switching its input on so the enable validates against the pin
    //    it will use.
    if let Some(want) = io.spdif_rx_pins.first().copied()
        && current_u8(session, "in.spdif.pin", &[0]) != Some(want)
    {
        attempt(
            session,
            report,
            "S/PDIF RX pin",
            "in.spdif.pin",
            &[0],
            Value::Int(want as i64),
        );
    }
    if has("spdif_multi_input") {
        let (count, mask) = spdif_input_config(session);
        for index in 1..count.min(4) {
            let want_pin = document_spdif_pin(io, index);
            let want_enabled = io.spdif_enabled_ext.map(|m| m & (1 << (index - 1)) != 0);
            let is_enabled = mask & (1 << index) != 0;
            let what = format!("S/PDIF input {}", index + 1);

            if is_enabled && want_enabled == Some(false) {
                attempt(
                    session,
                    report,
                    &what,
                    "in.spdif.enable",
                    &[index],
                    Value::Bool(false),
                );
            }
            if let Some(pin) = want_pin
                && current_u8(session, "in.spdif.pin", &[index]) != Some(pin)
            {
                attempt(
                    session,
                    report,
                    &format!("S/PDIF {} RX pin", index + 1),
                    "in.spdif.pin",
                    &[index],
                    Value::Int(pin as i64),
                );
            }
            if !is_enabled && want_enabled == Some(true) {
                attempt(
                    session,
                    report,
                    &what,
                    "in.spdif.enable",
                    &[index],
                    Value::Bool(true),
                );
            }
        }
    } else if io.spdif_enabled_ext.is_some_and(|m| m != 0) {
        report.skip("Additional S/PDIF inputs (not supported by this firmware)");
    }

    // 6. I2S input (swift:653-663): the channel count first, because lowering
    //    it frees pairs and never fails, then each pair's data pin, then the
    //    rate.
    if let Some(want) = io.i2s_input_channels
        && has("i2s_input_channels")
        && matches!(want, 2 | 4 | 6 | 8)
        && current_u8(session, "in.i2s.channels", &[]) != Some(want as u8)
    {
        attempt(
            session,
            report,
            "I2S input channels",
            "in.i2s.channels",
            &[],
            Value::Int(want as i64),
        );
    }
    for (pair, want) in io.i2s_rx_pins.iter().take(4).enumerate() {
        if current_u8(session, "in.i2s.pin", &[pair as u8]) == Some(*want) {
            continue;
        }
        attempt(
            session,
            report,
            &format!("I2S serial data {} pin", pair + 1),
            "in.i2s.pin",
            &[pair as u8],
            Value::Int(*want as i64),
        );
    }
    if let Some(want) = io.i2s_input_rate_hz {
        let _ = session.write("in.rate", &[], Value::Int(want as i64));
    }

    // 7. Clocking (swift:665-679). The slave pair is a dormant store accepted
    //    at any time, so it goes in before the mode that starts using it.
    if let Some(want) = io.i2s_clock_mode {
        if has("i2s_slave_clock") {
            if current_u8(session, "in.i2s.clock", &[]) != Some(want) {
                attempt(
                    session,
                    report,
                    "I2S clock mode",
                    "in.i2s.clock",
                    &[],
                    Value::Choice(want),
                );
            }
        } else if want != 0 {
            report.skip("I2S clock-slave mode (not supported by this firmware)");
        }
    }
    if let Some(want) = io.i2s_bck_pin_slave {
        // Role 1 selects the slave pair; the registry row addresses the
        // unified pair only, so this one goes out as a raw write-as-read
        // (config.h:334-336, wValue = (role << 8) | GPIO).
        let status = session.with_transport(|t| {
            t.control_in(
                dspi_proto::generated::opcodes::REQ_SET_I2S_BCK_PIN,
                (1 << 8) | want as u16,
                1,
            )
        });
        match status {
            Ok(d) if d.first().copied().unwrap_or(0) != 0 => {
                let code = d[0];
                report.skip(format!(
                    "I2S slave BCK pin rejected by the device: {}",
                    describe_pin_status(code)
                ));
            }
            Ok(_) => {}
            Err(e) => report.skip(format!("I2S slave BCK pin rejected by the device: {e}")),
        }
    }
    if let Some(want) = io.i2s_clock_pin_mode
        && current_u8(session, "i2s.clockpins", &[]) != Some(want)
    {
        attempt(
            session,
            report,
            "I2S clock pin mode",
            "i2s.clockpins",
            &[],
            Value::Choice(want),
        );
    }

    // 8. ADAT output (swift:683-691): the data pin first, since it re-routes
    //    under a muted restart while enabled, then the enable state.
    if has("adat_output") {
        if let Some(want) = io.adat_pin
            && current_u8(session, "adat.pin", &[]) != Some(want)
        {
            attempt(
                session,
                report,
                "ADAT output pin",
                "adat.pin",
                &[],
                Value::Int(want as i64),
            );
        }
        if let Some(want) = io.adat_enabled
            && current_bool(session, "adat.enable") != Some(want)
        {
            attempt(
                session,
                report,
                "ADAT output enable",
                "adat.enable",
                &[],
                Value::Bool(want),
            );
        }
    } else if io.adat_enabled == Some(true) {
        report.skip("ADAT output (not supported by this device)");
    }

    // 9. ADAT input (swift:693-705).
    if has("adat_input") {
        if let Some(want) = io.adat_input_pin
            && want != PIN_UNSET
            && current_u8(session, "in.adat.pin", &[]) != Some(want)
        {
            attempt(
                session,
                report,
                "ADAT input pin",
                "in.adat.pin",
                &[],
                Value::Int(want as i64),
            );
        }
        if let Some(want) = io.adat_input_clock_mode
            && current_u8(session, "in.adat.clock", &[]) != Some(want)
        {
            attempt(
                session,
                report,
                "ADAT input clock mode",
                "in.adat.clock",
                &[],
                Value::Choice(want),
            );
        }
        if let Some(want) = io.adat_input_enabled
            && current_bool(session, "in.adat.enable") != Some(want)
        {
            attempt(
                session,
                report,
                "ADAT input enable",
                "in.adat.enable",
                &[],
                Value::Bool(want),
            );
        }
    } else if io.adat_input_enabled == Some(true) {
        report.skip("ADAT input (not supported by this device)");
    }

    // 10. The external DAC's mute pin (swift:708-714).
    if let Some(mute) = &io.dac_hw_mute {
        if has("dac_hardware_mute") {
            let what = "External DAC hardware mute";
            match session.write("dev.dacmute", &[], Value::Bytes(mute.encode())) {
                Err(crate::WriteError::Unavailable { .. }) => {
                    report.skip(format!("{what} (not supported by this firmware)"));
                }
                Err(e) => report.skip(format!("{what} rejected by the device: {e}")),
                // The registry reads one byte for a packet parameter, so the
                // write path's own readback cannot confirm a 16-byte config
                // (config.h:441). Confirm it here instead.
                Ok(_) => {
                    if read_dac_mute(session).is_some_and(|live| live != *mute) {
                        report.skip(format!("{what} was not applied by the device"));
                    }
                }
            }
        } else {
            report.skip("External DAC hardware mute (not supported by this firmware)");
        }
    }
}

/// Read the wiring back off the device.
///
/// Only what the registry can actually read: a field with no reader is left
/// absent rather than defaulted, because [`apply_io`] treats absent as "the
/// document does not say" and a made-up default here would come back as a real
/// pin move on the next import. The slave BCK pin is the one such gap; it has
/// no scalar GET of its own (config.h:334-336 puts the role in the SET's
/// wValue only) and lives in the bulk snapshot, which Phase 2A decodes.
fn capture_io(r: &mut Reader) -> Result<IoBlock, CaptureError> {
    let caps = r.session.capabilities().clone();
    let has = |name: &str| caps.features.iter().any(|f| f.name == name && f.present);
    let slots = caps.num_outputs.saturating_sub(1) / 2;

    let mut io = IoBlock {
        output_slot_types: r.list("out.type", 0..slots)?,
        i2s_bck_pin: r.u8("i2s.bck", &[])?,
        mck_enabled: r.bool("i2s.mck", &[])?,
        mck_pin: r.u8("i2s.mck.pin", &[])?,
        mck_multiplier: r.u8("i2s.mck.mult", &[])?.map(mck_multiple),
        i2s_clock_mode: r.u8("in.i2s.clock", &[])?,
        i2s_clock_pin_mode: r.u8("i2s.clockpins", &[])?,
        i2s_input_channels: r.u8("in.i2s.channels", &[])?.map(i32::from),
        i2s_input_rate_hz: r.f32("in.rate", &[])?.map(|hz| hz as u32),
        ..Default::default()
    };

    // One pin output per slot, then the PDM sub (config.h:667-673).
    io.output_pins = r.list("out.pin", 0..slots.saturating_add(1))?;

    let spdif = capture_spdif_input_config(r)?;
    io.spdif_rx_pins = r.list("in.spdif.pin", 0..3)?;
    if spdif.is_some_and(|(count, _)| count > 3) {
        io.spdif_rx_pin4 = r.u8("in.spdif.pin", &[3])?;
    }
    if has("spdif_multi_input") {
        io.spdif_enabled_ext = match spdif {
            // Bit 0 of the wire mask is input 0, which is always on; the
            // document's mask starts at input 1 (config.h:454-457).
            Some((_, mask)) => Some(mask >> 1),
            None => r.image_spdif_mask()?,
        };
    }

    io.i2s_rx_pins = r.list("in.i2s.pin", 0..4)?;

    if has("adat_output") {
        io.adat_enabled = r.bool("adat.enable", &[])?;
        io.adat_pin = r.u8("adat.pin", &[])?;
    }
    if has("adat_input") {
        io.adat_input_enabled = r.bool("in.adat.enable", &[])?;
        io.adat_input_pin = r.u8("in.adat.pin", &[])?;
        io.adat_input_clock_mode = r.u8("in.adat.clock", &[])?;
    }
    if has("dac_hardware_mute") {
        io.dac_hw_mute = capture_dac_mute(r)?;
    }

    Ok(io)
}

/// The live 16-byte DAC mute config (config.h:441).
fn capture_dac_mute(r: &mut Reader) -> Result<Option<DacHwMuteBlock>, CaptureError> {
    let opcode = dspi_proto::generated::opcodes::REQ_GET_DAC_HW_MUTE_CONFIG;
    let read = r.session.with_transport(|t| t.control_in(opcode, 0, 16));
    if let Some(bytes) = r.answer("DAC mute", read)? {
        return Ok(DacHwMuteBlock::decode(&bytes));
    }
    Ok(r.image()?.map(|st| {
        let m = st.dac_hw_mute();
        DacHwMuteBlock {
            enabled: m.enabled,
            active_low: m.active_low,
            pin: m.pin,
            hold_ms: m.hold_ms,
            release_ms: m.release_ms,
        }
    }))
}

/// The live 16-byte DAC mute config (config.h:441).
fn read_dac_mute(session: &mut Session) -> Option<DacHwMuteBlock> {
    let bytes = session
        .with_transport(|t| {
            t.control_in(
                dspi_proto::generated::opcodes::REQ_GET_DAC_HW_MUTE_CONFIG,
                0,
                16,
            )
        })
        .ok()?;
    DacHwMuteBlock::decode(&bytes)
}

/// `PIN_RESET_TO_DEFAULT`, which for the ADAT input means "no pin"
/// (config.h:595-604).
const PIN_UNSET: u8 = 0xFF;

/// The document holds the master-clock multiple; the wire holds a selector.
fn mck_selector(multiple: i32) -> u8 {
    u8::from(multiple == 256)
}

/// The other direction, for capture.
fn mck_multiple(selector: u8) -> i32 {
    if selector == 1 { 256 } else { 128 }
}

/// Write one hardware setting and record a refusal instead of throwing it.
///
/// The status byte is the firmware's own verdict for these write-as-read
/// setters, so it is believed first; the readback is what catches the
/// parameters that answer no status at all.
fn attempt(
    session: &mut Session,
    report: &mut ApplyReport,
    what: &str,
    path: &str,
    indices: &[u8],
    value: Value,
) {
    let outcome = session.write(path, indices, value);
    let status = session.last_write_status();

    match (outcome, status) {
        (Ok(_), Some(0))
        | (Ok(Outcome::Confirmed(_) | Outcome::Accepted | Outcome::Triggered), None) => {}
        (Ok(_), Some(code)) => report.skip(format!(
            "{what} rejected by the device: {}",
            describe_pin_status(code)
        )),
        (Ok(Outcome::Rejected { .. }), None) => {
            report.skip(format!("{what} was not applied by the device"));
        }
        (Err(crate::WriteError::Unavailable { .. }), _) => {
            report.skip(format!("{what} (not supported by this device)"));
        }
        (Err(e), _) => report.skip(format!("{what} rejected by the device: {e}")),
    }
}

/// The Console's explanation for a `PIN_CONFIG_*` code (config.h:607-613).
///
/// The first two sentences are the Console's own
/// (`DSPi_ConsoleApp.swift:1988-1989`, minus the interface name it substitutes
/// there); the rest are the shorter phrases its preset-apply path uses
/// (`PresetDocumentTransfer.swift:724-732`).
pub fn describe_pin_status(code: u8) -> &'static str {
    match code as u16 {
        status::PIN_CONFIG_INVALID_PIN => {
            "A pin is out of range or lacks the required mux function"
        }
        status::PIN_CONFIG_PIN_IN_USE => "A pin is already claimed by another output or interface",
        status::PIN_CONFIG_INVALID_OUTPUT => "invalid output",
        status::PIN_CONFIG_OUTPUT_ACTIVE => "output is active",
        status::PIN_CONFIG_INVALID_PARAM => "invalid parameter",
        _ => "unknown status",
    }
}

fn current_u8(session: &mut Session, path: &str, indices: &[u8]) -> Option<u8> {
    session.read(path, indices).ok().and_then(|v| v.as_u8())
}

fn current_bool(session: &mut Session, path: &str) -> Option<bool> {
    session.read(path, &[]).ok().and_then(|v| v.as_bool())
}

/// The S/PDIF pin a document holds for an input index, or `None` when the
/// document predates that input. Index 3 lives in the additive field.
fn document_spdif_pin(io: &IoBlock, index: u8) -> Option<u8> {
    if index == 3 {
        return io.spdif_rx_pin4;
    }
    io.spdif_rx_pins.get(index as usize).copied()
}

/// `{count, enable_mask, gpio[0..3]}` (config.h:456-457). Bit 0 of the mask is
/// input 0, which is always on.
fn spdif_input_config(session: &mut Session) -> (u8, u8) {
    session
        .with_transport(|t| {
            t.control_in(
                dspi_proto::generated::opcodes::REQ_GET_SPDIF_INPUT_CONFIG,
                0,
                6,
            )
        })
        .ok()
        .map(|d| (d[0], d[1]))
        .unwrap_or((1, 1))
}

/// [`spdif_input_config`] for a capture: `None` when the device would not say.
fn capture_spdif_input_config(r: &mut Reader) -> Result<Option<(u8, u8)>, CaptureError> {
    let opcode = dspi_proto::generated::opcodes::REQ_GET_SPDIF_INPUT_CONFIG;
    let read = r.session.with_transport(|t| t.control_in(opcode, 0, 6));
    Ok(r.answer("S/PDIF inputs", read)?
        .and_then(|d| Some((*d.first()?, *d.get(1)?))))
}

/// The device's crossfeed pair map, `None` on firmware without one. No image
/// holds it where the firmware lacks it, so there is nothing to fall back on.
fn capture_pair_map(r: &mut Reader) -> Result<Option<PairMap>, CaptureError> {
    let read = r
        .session
        .with_transport(|t| Ok(crate::ext::read_pair_map(t)));
    match read {
        Ok(Ok(map)) => Ok(Some(map)),
        Ok(Err(crate::ext::ExtError::Transport(e))) | Err(e) => {
            r.answer("the crossfeed pair map", Err(e))
        }
        Ok(Err(_)) => Ok(None),
    }
}

/// A read failed, so the document would be missing what the device does hold.
/// Nothing is written: no export is better than half of one.
#[derive(Debug, thiserror::Error)]
#[error("could not read {what} from the device: {source}")]
pub struct CaptureError {
    /// What was being read: a parameter path and its indices, or a section.
    pub what: String,
    #[source]
    pub source: TransportError,
}

/// Whether a transport error is the device declining to report something, as
/// opposed to the read failing. A stall is firmware without that command (and
/// [`dspi_transport::with_busy_retry`] returns one only once its retries are
/// spent); `Denied` is the read-only guard or a hub's policy. Everything else,
/// a short reply included, is a link that cannot be trusted for the rest.
fn is_refusal(e: &TransportError) -> bool {
    matches!(
        e,
        TransportError::Stalled { .. } | TransportError::Denied(_)
    )
}

/// One capture's view of the device: asks, tells a refused read from a failed
/// one, and on a read-only device falls back on the mapped bulk image.
struct Reader<'a> {
    session: &'a mut Session,
    read_only: bool,
    /// `feat::*`: what the device's wire format carries in its image.
    features: u32,
    /// The image, read at most once; the inner `None` is a device that refused it.
    image: Option<Option<DeviceState>>,
}

impl<'a> Reader<'a> {
    fn new(session: &'a mut Session) -> Self {
        let wire = session.capabilities().wire_format;
        Self {
            session,
            read_only: u16::from(wire) != WIRE_FORMAT_VERSION,
            features: wire_compat::features(wire),
            image: None,
        }
    }

    /// `Ok(None)` for a refusal, the typed error for a failure.
    fn answer<T>(
        &self,
        what: &str,
        read: dspi_transport::Result<T>,
    ) -> Result<Option<T>, CaptureError> {
        match read {
            Ok(v) => Ok(Some(v)),
            Err(e) if is_refusal(&e) => Ok(None),
            Err(source) => Err(CaptureError {
                what: what.to_string(),
                source,
            }),
        }
    }

    /// The mapped image, on a read-only device that gives it up.
    fn image(&mut self) -> Result<Option<&DeviceState>, CaptureError> {
        if !self.read_only {
            return Ok(None);
        }
        if self.image.is_none() {
            let read = crate::compat::read_bulk_any(self.session.transport_mut());
            let packet = match read {
                Ok(image) => Some(image.packet),
                Err(crate::WriteError::Transport(e)) => self.answer("the bulk image", Err(e))?,
                // Nothing else leaves the transport as a failure.
                Err(_) => None,
            };
            let caps = self.session.capabilities().clone();
            self.image = Some(packet.map(|p| DeviceState::new(caps, p)));
        }
        Ok(self.image.as_ref().and_then(Option::as_ref))
    }

    /// One setting: what the device says, else what its image holds, else `None`.
    fn value(&mut self, path: &str, ix: &[u8]) -> Result<Option<Value>, CaptureError> {
        match self.session.read(path, ix) {
            Ok(v) => return Ok(Some(v)),
            Err(crate::WriteError::Transport(e)) => {
                self.answer::<()>(&format!("{path} {ix:?}"), Err(e))?;
            }
            // Not a question this device can be asked: no such channel, or a
            // parameter its platform lacks.
            Err(_) => {}
        }
        let features = self.features;
        Ok(self
            .image()?
            .and_then(|st| image_value(st, features, path, ix)))
    }

    fn f32(&mut self, path: &str, ix: &[u8]) -> Result<Option<f32>, CaptureError> {
        Ok(self.value(path, ix)?.and_then(|v| v.as_f32()))
    }

    fn bool(&mut self, path: &str, ix: &[u8]) -> Result<Option<bool>, CaptureError> {
        Ok(self.value(path, ix)?.and_then(|v| v.as_bool()))
    }

    fn u8(&mut self, path: &str, ix: &[u8]) -> Result<Option<u8>, CaptureError> {
        Ok(self.value(path, ix)?.and_then(|v| v.as_u8()))
    }

    fn i32(&mut self, path: &str) -> Result<Option<i32>, CaptureError> {
        Ok(self.u8(path, &[])?.map(i32::from))
    }

    fn mask(&mut self, path: &str) -> Result<Option<i32>, CaptureError> {
        Ok(match self.value(path, &[])? {
            Some(Value::Mask(m)) => Some(m as i32),
            _ => None,
        })
    }

    /// A list whose positions mean something, so it stops at the first index
    /// the device will not report rather than closing up behind it.
    fn list(&mut self, path: &str, indices: std::ops::Range<u8>) -> Result<Vec<u8>, CaptureError> {
        let mut out = Vec::new();
        for i in indices {
            match self.u8(path, &[i])? {
                Some(v) => out.push(v),
                None => break,
            }
        }
        Ok(out)
    }

    /// `{first..first + count}` of a bank, or nothing if any band goes
    /// unreported: [`apply`] flattens what a bank leaves out, and leaves alone a
    /// bank with no bands. `in_image` finds a band in the mapped image.
    fn bank(
        &mut self,
        channel: u8,
        first: u8,
        count: u8,
        in_image: fn(&DeviceState, u8, u8) -> Option<EqParamPacket>,
    ) -> Result<Vec<BandBlock>, CaptureError> {
        let mut bank = Vec::new();
        for offset in 0..count {
            let band = first.saturating_add(offset);
            let read = match self.session.read_band(channel, band) {
                Ok(p) => Some(p),
                Err(crate::WriteError::Transport(e)) => {
                    let what = format!("channel {channel} band {band}");
                    self.answer(&what, Err(e))?
                }
                Err(_) => None,
            };
            let packet = match read {
                Some(p) => Some(p),
                None => self.image()?.and_then(|st| in_image(st, channel, offset)),
            };
            match packet {
                Some(p) => bank.push(band_block(&p)),
                None => return Ok(Vec::new()),
            }
        }
        Ok(bank)
    }

    /// The S/PDIF enable mask from the image, in the document's form.
    fn image_spdif_mask(&mut self) -> Result<Option<u8>, CaptureError> {
        let features = self.features;
        Ok(self.image()?.and_then(|st| {
            (features & feat::I2S_CLOCK_MODE != 0)
                .then(|| st.input_config()?.spdif_rx_enabled_ext)
                .flatten()
        }))
    }
}

/// What a mapped image holds for a parameter. An image older than a section or
/// field holds only zeros there, so the `feat::*` of the device's format decide
/// first. `None` for what no image holds (the master-volume mode, say).
fn image_value(st: &DeviceState, features: u32, path: &str, ix: &[u8]) -> Option<Value> {
    use Value::{Bool as B, Choice as C, Float as F, Int as I, Mask as M};
    let since = |bit: u32| features & bit != 0;
    let at = ix.first().map(|i| usize::from(*i));
    let output = |o: Option<usize>| o.filter(|o| *o < 9).map(|o| st.output(o));
    let limiter = |o: Option<usize>| since(feat::LIMITER).then(|| st.limiter(o?)).flatten();
    let input = st.input_config();
    Some(match path {
        "bypass" => B(st.global().bypass),
        "vol.master" => F(st.master_volume_db()),
        "vol.user" => F(st.user_volume().0),
        "in.lg" => B(st.lg_sound_sync().enabled),
        "in.source" => C(input?.input_source),
        "pre" => F(st.preamp_db(at.filter(|i| *i < 8)?)),
        "ch.delay" => F(st.delay_ms(at?)),
        "out.gain" => F(output(at)?.gain_db),
        "out.mute" => B(output(at)?.mute),
        "out.enable" => B(output(at)?.enabled),
        "out.delay" => F(output(at)?.delay_ms),

        "limit.on" => B(limiter(at)?.enabled),
        "limit.threshold" => F(limiter(at)?.threshold_db),
        "limit.release" => F(limiter(at)?.release_ms),
        "limit.link" => C(limiter(at)?.link_group),

        "loud.on" => B(st.global().loudness_enabled),
        "loud.ref" => F(st.global().loudness_ref_spl),
        "loud.intensity" => F(st.global().loudness_intensity_pct),
        "loud.mask" if since(feat::LOUDNESS_MASK) => M(st.global().loudness_output_mask.into()),

        "cf.on" => B(st.crossfeed().enabled),
        "cf.preset" => C(st.crossfeed().preset),
        "cf.freq" => F(st.crossfeed().custom_fc),
        "cf.feed" => F(st.crossfeed().custom_feed_db),
        "cf.itd" => B(st.crossfeed().itd_enabled),
        "cf.outputs" if since(feat::CROSSFEED_MASK) => M(st.crossfeed().output_pair_mask.into()),

        "lev.on" => B(st.leveller().enabled),
        "lev.speed" => C(st.leveller().speed),
        "lev.lookahead" => B(st.leveller().lookahead),
        "lev.amount" => F(st.leveller().amount_pct),
        "lev.maxgain" => F(st.leveller().max_gain_db),
        "lev.gate" => F(st.leveller().gate_threshold_db),
        "lev.masks" if since(feat::LEVELLER_MASKS) => {
            let l = st.leveller();
            M(u32::from(l.detector_mask) | u32::from(l.apply_mask) << 8)
        }

        "bass.on" if since(feat::PSYBASS) => B(st.psybass().enabled),
        "bass.cutoff" if since(feat::PSYBASS) => F(st.psybass().cutoff_hz),
        "bass.harmonics" if since(feat::PSYBASS) => F(st.psybass().harmonics_db),
        "bass.drive" if since(feat::PSYBASS) => F(st.psybass().drive_db),
        "bass.character" if since(feat::PSYBASS) => F(st.psybass().character_pct),
        "bass.original" if since(feat::PSYBASS) => F(st.psybass().original_db),
        "bass.mask" if since(feat::PSYBASS) => M(st.psybass().output_mask.into()),

        "up.on" if since(feat::UPMIX) => B(st.upmix().enabled),
        "up.center_mode" if since(feat::UPMIX) => C(st.upmix().center_mode),
        "up.surround_mode" if since(feat::UPMIX) => C(st.upmix().surround_mode),
        "up.strength" if since(feat::UPMIX) => F(st.upmix().strength),
        "up.width" if since(feat::UPMIX) => F(st.upmix().center_width),
        "up.threshold" if since(feat::UPMIX) => F(st.upmix().threshold),
        "up.attack" if since(feat::UPMIX) => F(st.upmix().attack_ms),
        "up.release" if since(feat::UPMIX) => F(st.upmix().release_ms),
        "up.det_hpf" if since(feat::UPMIX) => F(st.upmix().det_hpf_hz),
        "up.sur_delay" if since(feat::UPMIX) => F(st.upmix().sur_delay_ms),
        "up.sur_hpf" if since(feat::UPMIX) => F(st.upmix().sur_hpf_hz),
        "up.sur_lpf" if since(feat::UPMIX) => F(st.upmix().sur_lpf_hz),
        "up.decorr" if since(feat::UPMIX) => F(st.upmix().decorr),
        "up.presence" if since(feat::UPMIX_PRESENCE) => F(st.upmix().presence_db),

        "sub.on" if since(feat::SUBHARM) => B(st.subharm().enabled),
        "sub.low" if since(feat::SUBHARM) => F(st.subharm().low_db),
        "sub.high" if since(feat::SUBHARM) => F(st.subharm().high_db),
        "sub.boost" if since(feat::SUBHARM) => F(st.subharm().boost_db),
        "sub.mask" if since(feat::SUBHARM) => M(st.subharm().output_mask.into()),
        "sub.top" if since(feat::SUBHARM_EXT) => F(st.subharm().top_db),
        "sub.select" if since(feat::SUBHARM_EXT) => C(st.subharm().select_mode),
        "sub.depth" if since(feat::SUBHARM_EXT) => F(st.subharm().select_depth_pct),
        "sub.hold" if since(feat::SUBHARM_EXT) => F(st.subharm().select_hold_ms),
        "sub.ceiling" if since(feat::SUBHARM_EXT) => F(st.subharm().ceiling_db),
        "sub.link" if since(feat::SUBHARM_EXT) => B(st.subharm().link_pairs),

        "tube.on" if since(feat::TUBE) => B(st.tube().enabled),
        "tube.mask" if since(feat::TUBE) => M(st.tube().output_mask.into()),
        "tube.type" if since(feat::TUBE) => C(st.tube().tube_type),
        "tube.drive" if since(feat::TUBE) => F(st.tube().drive_db),
        "tube.bias" if since(feat::TUBE) => F(st.tube().bias_pct),
        "tube.asym" if since(feat::TUBE) => F(st.tube().asym_db),
        "tube.hardness" if since(feat::TUBE) => F(st.tube().hardness_pct),
        "tube.sag" if since(feat::TUBE) => F(st.tube().sag_pct),
        "tube.rectifier" if since(feat::TUBE) => C(st.tube().rectifier),
        "tube.xfmr" if since(feat::TUBE) => B(st.tube().xfmr_enabled),
        "tube.damping" if since(feat::TUBE) => F(st.tube().xfmr_damping),
        "tube.resonance" if since(feat::TUBE) => F(st.tube().xfmr_res_hz),
        "tube.mix" if since(feat::TUBE) => F(st.tube().mix_pct),
        "tube.trim" if since(feat::TUBE) => F(st.tube().trim_db),

        "out.type" => C(*st.i2s().output_types.get(at?)?),
        "out.pin" => C(*st.output_pins().get(at?)?),
        "i2s.bck" => C(st.i2s().bck_pin),
        "i2s.mck" => B(st.i2s().mck_enabled),
        "i2s.mck.pin" => C(st.i2s().mck_pin),
        "i2s.mck.mult" => C(st.i2s().mck_multiplier),
        "i2s.clockpins" if since(feat::I2S_CLOCK_MODE) => C(st.i2s().clock_pin_mode?),
        "in.i2s.clock" if since(feat::I2S_CLOCK_MODE) => C(input?.i2s_clock_mode),
        "in.i2s.channels" => C(input?.i2s_input_channels?),
        "in.rate" => I(match input?.i2s_input_rate {
            0 => 44_100,
            1 => 48_000,
            2 => 96_000,
            _ => return None,
        }),
        "in.spdif.pin" => match at? {
            0 => C(input?.spdif_rx_pin),
            3 if since(feat::SPDIF_IN4) => C(input?.spdif_rx_pin_ext[2]?),
            i @ (1 | 2) if since(feat::I2S_CLOCK_MODE) => {
                C(input?.spdif_rx_pin_ext[i.checked_sub(1)?]?)
            }
            _ => return None,
        },
        "in.i2s.pin" => match at? {
            0 => C(input?.i2s_rx_pin),
            i @ 1..=3 => C(input?.i2s_rx_pin_ext[i.checked_sub(1)?]?),
            _ => return None,
        },
        "adat.enable" if since(feat::ADAT_OUT) => B(st.adat_output().0),
        "adat.pin" if since(feat::ADAT_OUT) => C(st.adat_output().1),
        "in.adat.enable" if since(feat::ADAT_IN) => B(input?.adat_input_enabled?),
        "in.adat.pin" if since(feat::ADAT_IN) => C(input?.adat_input_pin?),
        "in.adat.clock" if since(feat::ADAT_IN) => C(input?.adat_clock_mode?),
        _ => return None,
    })
}

/// Capture the current device state as a document, in the Console's shape
/// (PresetDocumentTransfer.swift:20-197): every channel carries the Windows
/// id as `channelId` and the Console's own index fields beside it, so either
/// Console places it where it came from.
///
/// A value in the document is one the device reported. A read that fails (the
/// link is gone, timed out, or answers short) fails the capture with
/// [`CaptureError`]. A read the device refuses means it does not report that
/// setting: a read-only device's mapped image answers if it holds the value, and
/// otherwise the key is omitted ([`omitted_settings`] lists them).
pub fn capture(
    session: &mut Session,
    name: Option<String>,
) -> Result<PresetDocument, CaptureError> {
    let caps = session.capabilities().clone();
    let has = |name: &str| caps.features.iter().any(|f| f.name == name && f.present);
    let mut r = Reader::new(session);

    let mut channels = Vec::new();
    for c in &caps.channels {
        let channel_ref = if c.is_output {
            ChannelRef::Output(c.index.saturating_sub(caps.num_inputs))
        } else {
            ChannelRef::Input(c.index)
        };
        let mut block = ChannelBlock {
            name: c.name.clone(),
            is_output: c.is_output,
            eq_channel: Some(c.index as i32),
            delay_ms: r.f32("ch.delay", &[c.index])?,
            ..Default::default()
        };
        match channel_ref {
            ChannelRef::Input(i) => {
                block.channel_id = channel_id::for_input(i);
                block.input_index = Some(i as i32);
                // An input has no strip, so these say nothing about it. The
                // Console writes them all the same (its fields are not optional),
                // and so does this, to keep the two documents' shapes alike.
                block.gain_db = Some(0.0);
                block.muted = Some(false);
                block.enabled = Some(true);
            }
            ChannelRef::Output(out) => {
                block.channel_id = channel_id::for_output(out);
                block.output_index = Some(out as i32);
                block.gain_db = r.f32("out.gain", &[out])?;
                block.muted = r.bool("out.mute", &[out])?;
                block.enabled = r.bool("out.enable", &[out])?;
                block.output_delay_ms = r.f32("out.delay", &[out])?;
                if has("output_limiter") {
                    let limiter = LimiterBlock {
                        enabled: r.bool("limit.on", &[out])?,
                        threshold_db: r.f32("limit.threshold", &[out])?,
                        release_ms: r.f32("limit.release", &[out])?,
                        link_group: r.u8("limit.link", &[out])?.map(i32::from),
                    };
                    block.limiter = (!says_nothing(&limiter)).then_some(limiter);
                }
            }
        }
        block.eq = r.bank(c.index, 0, caps.max_bands, DeviceState::band)?;
        if c.is_output {
            block.crossover = r.bank(
                c.index,
                CROSSOVER_FIRST_BAND,
                CROSSOVER_BANDS,
                DeviceState::xover_band,
            )?;
        }
        channels.push(block);
    }

    // Eight long, as the Console writes it, unless an input goes unreported:
    // a position is an input, so the list stops there.
    let mut preamps = vec![0.0f32; 8];
    for i in 0..caps.num_inputs.min(8) {
        match r.f32("pre", &[i])? {
            Some(db) => preamps[i as usize] = db,
            None => {
                preamps.truncate(i as usize);
                break;
            }
        }
    }

    // A read-only device's matrix comes from its image, which is where the
    // strict bulk reader would find it anyway.
    let point = |i: usize, o: usize, enabled, invert, gain_db| CrosspointBlock {
        input: i as i32,
        output: o as i32,
        enabled,
        invert,
        gain_db,
    };
    let (inputs, outputs) = (caps.num_inputs as usize, caps.num_outputs as usize);
    let mut matrix = Vec::new();
    if r.read_only {
        if let Some(st) = r.image()? {
            for i in 0..inputs {
                for o in 0..outputs {
                    let c = st.crosspoint(i, o);
                    matrix.push(point(i, o, c.enabled, c.phase_invert, c.gain_db));
                }
            }
        }
    } else {
        let read = match r.session.read_matrix() {
            Ok((grid, _)) => Some(grid),
            Err(crate::WriteError::Transport(e)) => r.answer("the matrix", Err(e))?,
            Err(_) => None,
        };
        for (i, row) in read.unwrap_or_default().iter().enumerate() {
            for (o, c) in row.iter().enumerate() {
                matrix.push(point(i, o, c.enabled, c.phase_invert, c.gain_db));
            }
        }
    }

    let io = capture_io(&mut r)?;
    // Only from a device that has the map (PresetDocumentTransfer.swift:57-59),
    // so a stock device's document is unchanged.
    let pair_map = capture_pair_map(&mut r)?.map(|m| m.0.iter().map(|b| *b as i32).collect());

    // `[detector, apply]` in one word (config.h:441).
    let lev_masks = if has("leveller_masks") {
        r.mask("lev.masks")?
    } else {
        None
    };
    let meta = Meta {
        name,
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        platform: Some(caps.platform.name()),
        firmware_version: Some(caps.firmware.clone()),
        wire_format_version: caps.wire_format as i32,
        input_channel_count: caps.num_inputs as i32,
        output_channel_count: caps.num_outputs as i32,
        saved_utc: Some(utc_now()),
        master_volume_mode: r.i32("vol.master.mode")?,
        output_config_mode: r.i32("preset.iomode")?,
    };
    let global = GlobalBlock {
        input_preamps_db: preamps,
        bypass: r.bool("bypass", &[])?,
        master_volume_db: r.f32("vol.master", &[])?,
        user_volume_db: r.f32("vol.user", &[])?,
        input_source: r.u8("in.source", &[])?,
        lg_sound_sync_enabled: r.bool("in.lg", &[])?,
        // The app's own; the caller fills it in (see `apply`).
        input_pair_linked: vec![false; 4],
    };
    let loudness = LoudnessBlock {
        enabled: r.bool("loud.on", &[])?,
        ref_spl: r.f32("loud.ref", &[])?,
        intensity_pct: r.f32("loud.intensity", &[])?,
        // Where the device has no mask, the document does not claim one.
        output_mask: if has("loudness_output_mask") {
            r.mask("loud.mask")?
        } else {
            None
        },
    };
    let crossfeed = CrossfeedBlock {
        enabled: r.bool("cf.on", &[])?,
        preset: r.i32("cf.preset")?,
        freq_hz: r.f32("cf.freq", &[])?,
        feed_db: r.f32("cf.feed", &[])?,
        itd: r.bool("cf.itd", &[])?,
        output_pair_mask: if has("crossfeed_output_mask") {
            r.mask("cf.outputs")?.map(|m| m & 0xFF)
        } else {
            None
        },
        pair_map,
    };
    let leveller = LevellerBlock {
        enabled: r.bool("lev.on", &[])?,
        speed: r.i32("lev.speed")?,
        lookahead: r.bool("lev.lookahead", &[])?,
        amount_pct: r.f32("lev.amount", &[])?,
        max_gain_db: r.f32("lev.maxgain", &[])?,
        gate_db: r.f32("lev.gate", &[])?,
        detector_mask: lev_masks.map(|m| m & 0xFF),
        apply_mask: lev_masks.map(|m| (m >> 8) & 0xFF),
    };
    // Absent rather than defaulted: a document must not claim a device had a
    // feature it does not. Nor a block with nothing in it, which reads the same.
    let psybass = if has("psychoacoustic_bass") {
        Some(PsybassBlock {
            enabled: r.bool("bass.on", &[])?,
            cutoff_hz: r.f32("bass.cutoff", &[])?,
            harmonics_db: r.f32("bass.harmonics", &[])?,
            drive_db: r.f32("bass.drive", &[])?,
            character_pct: r.f32("bass.character", &[])?,
            original_db: r.f32("bass.original", &[])?,
            output_mask: r.mask("bass.mask")?,
        })
    } else {
        None
    };
    let upmix = if has("upmixer") {
        Some(UpmixBlock {
            enabled: r.bool("up.on", &[])?,
            center_mode: r.i32("up.center_mode")?,
            surround_mode: r.i32("up.surround_mode")?,
            strength_pct: r.f32("up.strength", &[])?,
            center_width_pct: r.f32("up.width", &[])?,
            threshold_pct: r.f32("up.threshold", &[])?,
            attack_ms: r.f32("up.attack", &[])?,
            release_ms: r.f32("up.release", &[])?,
            detector_hpf_hz: r.f32("up.det_hpf", &[])?,
            surround_delay_ms: r.f32("up.sur_delay", &[])?,
            surround_hpf_hz: r.f32("up.sur_hpf", &[])?,
            surround_lpf_hz: r.f32("up.sur_lpf", &[])?,
            decorr_pct: r.f32("up.decorr", &[])?,
            presence_db: r.f32("up.presence", &[])?,
        })
    } else {
        None
    };
    // Every field, as the Console exports them (PresetDocumentTransfer.
    // swift:100-137).
    let subharm = if has("subharmonic_synth") {
        Some(SubharmBlock {
            enabled: r.bool("sub.on", &[])?,
            low_db: r.f32("sub.low", &[])?,
            high_db: r.f32("sub.high", &[])?,
            top_db: r.f32("sub.top", &[])?,
            boost_db: r.f32("sub.boost", &[])?,
            output_mask: r.mask("sub.mask")?,
            select_mode: r.i32("sub.select")?,
            select_depth_pct: r.f32("sub.depth", &[])?,
            select_hold_ms: r.f32("sub.hold", &[])?,
            ceiling_db: r.f32("sub.ceiling", &[])?,
            link_pairs: r.bool("sub.link", &[])?,
        })
    } else {
        None
    };
    let tube = if has("tube_preamp") {
        Some(TubeBlock {
            enabled: r.bool("tube.on", &[])?,
            output_mask: r.mask("tube.mask")?,
            tube_type: r.i32("tube.type")?,
            drive_db: r.f32("tube.drive", &[])?,
            bias_pct: r.f32("tube.bias", &[])?,
            asym_db: r.f32("tube.asym", &[])?,
            hardness_pct: r.f32("tube.hardness", &[])?,
            sag_pct: r.f32("tube.sag", &[])?,
            rectifier: r.i32("tube.rectifier")?,
            xfmr_enabled: r.bool("tube.xfmr", &[])?,
            xfmr_damping: r.f32("tube.damping", &[])?,
            xfmr_res_hz: r.f32("tube.resonance", &[])?,
            mix_pct: r.f32("tube.mix", &[])?,
            trim_db: r.f32("tube.trim", &[])?,
        })
    } else {
        None
    };

    Ok(PresetDocument {
        schema_version: CURRENT_SCHEMA_VERSION,
        meta,
        global,
        loudness,
        crossfeed,
        leveller,
        psybass: psybass.filter(|b| !says_nothing(b)),
        upmix: upmix.filter(|b| !says_nothing(b)),
        subharm: subharm.filter(|b| !says_nothing(b)),
        tube: tube.filter(|b| !says_nothing(b)),
        channels,
        matrix,
        io,
    })
}

/// The current time as the ISO-8601 UTC string the Console writes
/// (`ISO8601DateFormatter`, PresetDocumentTransfer.swift:25), such as
/// `2026-09-30T10:00:00Z`.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601(secs)
}

/// Seconds since the epoch as an ISO-8601 UTC timestamp, by the civil-from-
/// days algorithm (Howard Hinnant's), which needs no calendar crate.
fn iso8601(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn band_block(p: &EqParamPacket) -> BandBlock {
    BandBlock {
        r#type: p.filter_type.to_raw() as i32,
        freq_hz: p.freq,
        q: p.q,
        gain: p.gain_db,
        qp: p.qp.unwrap_or(0.707),
        bypass: p.bypass,
    }
}

// ---------------------------------------------------------------------------
// WP TM10: the crossfeed pair map, filter banks as JSON, control surfaces
// ---------------------------------------------------------------------------

/// The largest document the Terminal reads, from a file or standard input.
/// Same limit as `dspi_core::files::MAX_FILE_BYTES`: a full export of a
/// 17-channel device is about 100 KB.
pub const MAX_DOCUMENT_BYTES: usize = 4 << 20;

/// The device's crossfeed pair map, or `None` on firmware without one.
fn read_pair_map(session: &mut Session) -> Option<PairMap> {
    session
        .with_transport(|t| Ok(crate::ext::read_pair_map(t).ok()))
        .ok()
        .flatten()
}

/// Apply a document's pair map, as the Console does
/// (PresetDocumentTransfer.swift:598-604): firmware without the map reports
/// only a map that asks for something other than the default.
fn apply_pair_map(session: &mut Session, doc: &PresetDocument, report: &mut ApplyReport) {
    let pairs = (session.capabilities().num_outputs / 2).min(PairMap::PAIRS as u8);
    let Some(map) = doc.crossfeed.pair_map_for(pairs) else {
        return;
    };
    if read_pair_map(session).is_none() {
        if map.normalized() != PairMap([0; PairMap::SIZE]) {
            report.skip("Crossfeed output pairs (not supported by this firmware)");
        }
        return;
    }
    // This goes straight to the transport, so a dry run stops here itself.
    if session.dry_run {
        return;
    }
    let written = session.with_transport(|t| Ok(crate::ext::write_pair_map(t, map)));
    if !matches!(written, Ok(Ok(_))) {
        report.skip("Crossfeed output pairs (the device refused the map)");
    }
}

/// Which document a text is, from its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// A `.dspipreset` whole-device document.
    Preset,
    /// A [`FilterDocument`].
    FilterJson,
    /// A [`ControlSurfaceDocument`].
    ControlSurfaces,
    /// Anything that is not a JSON object: a filter text file.
    FilterText,
}

/// Look at a text and say what it is. The filter text grammar has no `{`
/// lead, so a JSON object is never a filter file. An object naming no known
/// `kind` is taken for a preset, whose parser then gives the real reason.
pub fn sniff(text: &str) -> DocumentKind {
    #[derive(Deserialize)]
    struct Kind {
        #[serde(default)]
        kind: Option<String>,
    }
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if !text.starts_with('{') {
        return DocumentKind::FilterText;
    }
    match serde_json::from_str::<Kind>(text).ok().and_then(|k| k.kind) {
        Some(k) if k == FILTER_DOCUMENT_KIND => DocumentKind::FilterJson,
        Some(k) if k == CS_DOCUMENT_KIND => DocumentKind::ControlSurfaces,
        _ => DocumentKind::Preset,
    }
}

/// `kind` of a [`FilterDocument`].
pub const FILTER_DOCUMENT_KIND: &str = "dspi-filters";

/// A filter file as JSON: the same content as the text format
/// ([`crate::filterfile`]), one entry per section, with bands in the
/// `.dspipreset` spelling ([`BandBlock`]). No other client writes this yet;
/// it is the Terminal's scripting form of the text file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterDocument {
    pub kind: String,
    /// The text format's version ([`crate::filterfile::FORMAT_VERSION`]).
    pub format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_utc: Option<String>,
    #[serde(default)]
    pub channels: Vec<FilterChannel>,
}

/// One channel's filters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterChannel {
    /// The section header, as the text format has it.
    pub header: String,
    /// Input or output index, when the header gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    pub is_output: bool,
    /// Inputs only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamp_db: Option<f32>,
    pub peq: Vec<BandBlock>,
    pub crossover: Vec<BandBlock>,
}

/// Parse a [`FilterDocument`], refusing the wrong `kind`, a newer format and
/// an empty one.
pub fn parse_filters(text: &str) -> Result<FilterDocument, PresetFileError> {
    use crate::filterfile::FORMAT_VERSION;
    if text.trim().is_empty() {
        return Err(PresetFileError::Empty);
    }
    let doc: FilterDocument = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| PresetFileError::Invalid(e.to_string()))?;
    if doc.kind != FILTER_DOCUMENT_KIND {
        return Err(PresetFileError::Invalid(format!(
            "kind `{}` is not `{FILTER_DOCUMENT_KIND}`",
            doc.kind
        )));
    }
    if doc.format_version > FORMAT_VERSION {
        return Err(PresetFileError::FromTheFuture {
            got: doc.format_version as i32,
            supported: FORMAT_VERSION as i32,
        });
    }
    if doc.channels.is_empty() {
        return Err(PresetFileError::NoChannels);
    }
    Ok(doc)
}

impl FilterDocument {
    /// The JSON text.
    pub fn write(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// From a parsed text file. The text carries no `qp`, so it is the default.
    pub fn from_text(file: &crate::filterfile::FilterFile) -> Self {
        let band = |b: &dspi_proto::dsp::Band| BandBlock {
            r#type: b.filter_type.to_raw() as i32,
            freq_hz: b.freq,
            q: b.q,
            gain: b.gain_db,
            qp: BandBlock::default().qp,
            bypass: b.bypass,
        };
        Self {
            kind: FILTER_DOCUMENT_KIND.into(),
            format_version: file.format_version,
            saved_utc: None,
            channels: file
                .channels
                .iter()
                .map(|c| FilterChannel {
                    header: c.header.clone(),
                    index: c.index,
                    is_output: c.is_output,
                    preamp_db: c.preamp_db,
                    peq: c.peq.iter().map(band).collect(),
                    crossover: c.crossover.iter().map(band).collect(),
                })
                .collect(),
        }
    }

    /// What this document leaves out because the device would not report it, as
    /// `header: what` for each channel. Empty when it holds everything.
    pub fn unreported(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in &self.channels {
            let mut missing = Vec::new();
            if !c.is_output && c.preamp_db.is_none() {
                missing.push("preamp");
            }
            if c.peq.is_empty() {
                missing.push("EQ");
            }
            if c.is_output && c.crossover.is_empty() {
                missing.push("crossover");
            }
            if !missing.is_empty() {
                out.push(format!("{}: {}", c.header, missing.join(", ")));
            }
        }
        out
    }

    /// As a text file.
    pub fn to_text(&self) -> crate::filterfile::FilterFile {
        let band = |b: &BandBlock| dspi_proto::dsp::Band {
            filter_type: dspi_proto::FilterType::from_raw(b.r#type.clamp(0, 255) as u8),
            freq: b.freq_hz,
            q: b.q,
            gain_db: b.gain,
            bypass: b.bypass,
        };
        crate::filterfile::FilterFile {
            format_version: crate::filterfile::FORMAT_VERSION,
            channels: self
                .channels
                .iter()
                .map(|c| crate::filterfile::ChannelBank {
                    header: c.header.clone(),
                    index: c.index,
                    is_output: c.is_output,
                    preamp_db: c.preamp_db,
                    peq: c.peq.iter().map(band).collect(),
                    crossover: c.crossover.iter().map(band).collect(),
                })
                .collect(),
        }
    }
}

/// Read every channel's filters from the device, by [`capture`]'s rule: a failed
/// read fails it with [`CaptureError`], and a refused one falls back on a
/// read-only device's mapped image. A bank is whole or absent, since a position
/// in it is a band, and an import leaves an absent bank alone
/// ([`FilterDocument::unreported`] lists them).
pub fn capture_filters(session: &mut Session) -> Result<FilterDocument, CaptureError> {
    let caps = session.capabilities().clone();
    let mut r = Reader::new(session);
    let mut channels = Vec::with_capacity(caps.channels.len());
    for c in &caps.channels {
        let index = if c.is_output {
            c.index.saturating_sub(caps.num_inputs)
        } else {
            c.index
        };
        let mut channel = FilterChannel {
            header: format!(
                "{} {index}: {}",
                if c.is_output { "Output" } else { "Input" },
                c.name
            ),
            index: Some(index),
            is_output: c.is_output,
            ..Default::default()
        };
        if !c.is_output {
            channel.preamp_db = r.f32("pre", &[c.index])?;
        }
        channel.peq = r.bank(c.index, 0, caps.max_bands, DeviceState::band)?;
        if c.is_output {
            channel.crossover = r.bank(
                c.index,
                CROSSOVER_FIRST_BAND,
                CROSSOVER_BANDS,
                DeviceState::xover_band,
            )?;
        }
        channels.push(channel);
    }
    Ok(FilterDocument {
        kind: FILTER_DOCUMENT_KIND.into(),
        format_version: crate::filterfile::FORMAT_VERSION,
        saved_utc: Some(utc_now()),
        channels,
    })
}

/// What a filter import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterReport {
    pub bands_applied: usize,
    pub crossover_bands_applied: usize,
    pub preamps_applied: usize,
    /// Writes the device or the session refused.
    pub refused: usize,
    /// Bands past what this device has, and crossover bands on an input.
    pub dropped: usize,
}

/// Apply one channel's filters to unified channel `channel`, through the
/// ordinary write path (clamping, gating, readback, dry run).
pub fn apply_filter_channel(
    session: &mut Session,
    channel: u8,
    filters: &FilterChannel,
    report: &mut FilterReport,
) {
    let caps = session.capabilities().clone();
    let is_output = channel >= caps.num_inputs;
    if let Some(db) = filters.preamp_db {
        if is_output {
            report.dropped += 1;
        } else if session.write("pre", &[channel], Value::Float(db)).is_ok() {
            report.preamps_applied += 1;
        } else {
            report.refused += 1;
        }
    }
    for (i, b) in filters.peq.iter().enumerate() {
        if i >= caps.max_bands as usize {
            report.dropped += 1;
        } else if apply_band(session, channel, i as u8, b).is_some() {
            report.bands_applied += 1;
        } else {
            report.refused += 1;
        }
    }
    for (i, b) in filters.crossover.iter().enumerate() {
        if !is_output || i >= CROSSOVER_BANDS as usize {
            report.dropped += 1;
        } else if apply_band(session, channel, CROSSOVER_FIRST_BAND + i as u8, b).is_some() {
            report.crossover_bands_applied += 1;
        } else {
            report.refused += 1;
        }
    }
}

// ------------------------------------------------------------ control surfaces

/// `kind` of a [`ControlSurfaceDocument`].
pub const CS_DOCUMENT_KIND: &str = "dspi-control-surfaces";

/// Bumped when the control-surface document changes incompatibly.
pub const CS_SCHEMA_VERSION: i32 = 1;

/// A device's control-surface configuration: bindings and their names, IR
/// commands, groups, macros and the display.
///
/// Neither the Mac nor Qt has a file for this, and `.dspipreset` has no such
/// block in any client, so this is its own document rather than a block a
/// preset import would act on: bindings claim GPIOs, and the Consoles would
/// silently drop the block on the way through. The keys are those of
/// `dspi_core`'s `cs_snapshot`, the subset that is configuration rather than
/// capability or live status, so a future Qt or Swift importer can hand them to
/// `cs_apply` as they are. Entries are by position: the first is slot 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSurfaceDocument {
    pub kind: String,
    pub schema_version: i32,
    #[serde(default)]
    pub meta: CsMeta,
    #[serde(default)]
    pub bindings: Vec<CsBindingBlock>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub ir: Vec<CsIrBlock>,
    #[serde(default)]
    pub groups: Vec<CsGroupBlock>,
    #[serde(default)]
    pub macros: Vec<CsMacroBlock>,
    /// Absent on firmware without the display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<CsDisplayBlock>,
}

/// Provenance, informational only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    pub caps_version: u8,
}

/// One binding slot, in `cs_snapshot`'s spelling. Pin numbers are a board's;
/// that is why this document is only ever applied on request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsBindingBlock {
    pub r#type: u8,
    pub noun: u8,
    pub action: u8,
    pub flags: u8,
    pub gpio0: u8,
    pub gpio1: u8,
    pub event: u8,
    pub target: u8,
    pub index: u8,
    pub base_bright: u8,
    pub value: i16,
    pub step: i16,
    pub range_min: i16,
    pub range_max: i16,
    pub on_delay: u16,
    pub off_delay: u16,
    pub extras: u8,
}

impl Default for CsBindingBlock {
    fn default() -> Self {
        Self::from(&CsBinding {
            gpio: [0, GPIO_UNUSED],
            ..Default::default()
        })
    }
}

impl From<&CsBinding> for CsBindingBlock {
    fn from(b: &CsBinding) -> Self {
        Self {
            r#type: b.component,
            noun: b.noun,
            action: b.action,
            flags: b.flags,
            gpio0: b.gpio[0],
            gpio1: b.gpio[1],
            event: b.event,
            target: b.target,
            index: b.index,
            base_bright: b.base_bright,
            value: b.value,
            step: b.step,
            range_min: b.range_min,
            range_max: b.range_max,
            on_delay: b.on_delay,
            off_delay: b.off_delay,
            extras: b.extras,
        }
    }
}

impl From<&CsBindingBlock> for CsBinding {
    fn from(b: &CsBindingBlock) -> Self {
        Self {
            component: b.r#type,
            noun: b.noun,
            action: b.action,
            flags: b.flags,
            gpio: [b.gpio0, b.gpio1],
            event: b.event,
            target: b.target,
            index: b.index,
            base_bright: b.base_bright,
            value: b.value,
            step: b.step,
            range_min: b.range_min,
            range_max: b.range_max,
            on_delay: b.on_delay,
            off_delay: b.off_delay,
            extras: b.extras,
        }
    }
}

/// One IR sub-slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsIrBlock {
    pub noun: u8,
    pub action: u8,
    pub flags: u8,
    pub target: u8,
    pub index: u8,
    pub protocol: u8,
    pub value: i16,
    pub step: i16,
    pub code: u32,
}

/// One channel group.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsGroupBlock {
    pub target_kind: u8,
    pub member_mask: u32,
    pub name: String,
}

/// One macro: its active steps only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsMacroBlock {
    pub name: String,
    pub steps: Vec<CsStepBlock>,
}

/// One macro step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsStepBlock {
    pub noun: u8,
    pub action: u8,
    pub flags: u8,
    pub target: u8,
    pub index: u8,
    pub value: i16,
    pub step: i16,
    pub pre_delay: u16,
}

/// The display's configuration and pages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsDisplayBlock {
    pub config: CsDisplayConfigBlock,
    pub pages: Vec<CsPageBlock>,
}

/// `REQ_SET_CS_DISPLAY_CFG`'s record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsDisplayConfigBlock {
    pub mode: u8,
    pub home_page: u8,
    pub dwell: u16,
    pub overlay_hold: u16,
    pub brightness: u8,
    pub flags: u8,
    pub edit_timeout: u16,
}

/// One display page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsPageBlock {
    pub noun: u8,
    pub target: u8,
    pub index: u8,
    pub flags: u8,
}

fn ir_command(b: &CsIrBlock) -> IrCommand {
    IrCommand {
        noun: b.noun,
        action: b.action,
        flags: b.flags,
        target: b.target,
        index: b.index,
        protocol: b.protocol,
        value: b.value,
        step: b.step,
        code: b.code,
    }
}

fn macro_step(b: &CsStepBlock) -> CsMacroStep {
    CsMacroStep {
        noun: b.noun,
        action: b.action,
        flags: b.flags,
        target: b.target,
        index: b.index,
        value: b.value,
        step: b.step,
        pre_delay: b.pre_delay,
    }
}

fn display_cfg(b: &CsDisplayConfigBlock) -> CsDisplayCfg {
    CsDisplayCfg {
        mode: b.mode,
        home_page: b.home_page,
        dwell: b.dwell,
        overlay_hold: b.overlay_hold,
        brightness: b.brightness,
        flags: b.flags,
        edit_timeout: b.edit_timeout,
    }
}

/// Parse a [`ControlSurfaceDocument`], refusing the wrong `kind` and a newer
/// schema. A number out of its field's range is a parse error, not a wrap.
pub fn parse_control_surfaces(text: &str) -> Result<ControlSurfaceDocument, PresetFileError> {
    if text.trim().is_empty() {
        return Err(PresetFileError::Empty);
    }
    let doc: ControlSurfaceDocument = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| PresetFileError::Invalid(e.to_string()))?;
    if doc.kind != CS_DOCUMENT_KIND {
        return Err(PresetFileError::Invalid(format!(
            "kind `{}` is not `{CS_DOCUMENT_KIND}`",
            doc.kind
        )));
    }
    if doc.schema_version <= 0 {
        return Err(PresetFileError::Invalid(
            "missing or invalid schemaVersion".into(),
        ));
    }
    if doc.schema_version > CS_SCHEMA_VERSION {
        return Err(PresetFileError::FromTheFuture {
            got: doc.schema_version,
            supported: CS_SCHEMA_VERSION,
        });
    }
    Ok(doc)
}

impl ControlSurfaceDocument {
    /// The JSON text.
    pub fn write(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Why a control-surface document could not be read from or applied to a
/// device at all. Per-record refusals are in [`CsApplyReport`] instead.
#[derive(Debug, thiserror::Error)]
pub enum CsFileError {
    #[error("this device has no control surfaces")]
    Unsupported,
    #[error(transparent)]
    Device(#[from] ApplyError),
}

/// Read the device's control-surface configuration.
///
/// # Errors
/// [`CsFileError::Unsupported`] on firmware without control surfaces;
/// [`CsFileError::Device`] when the link fails. [`CsData::read`] reads a
/// missing part as empty, so a final status read tells that apart from a link
/// that went away, rather than exporting a blank configuration.
pub fn capture_control_surfaces(
    session: &mut Session,
) -> Result<ControlSurfaceDocument, CsFileError> {
    let data = CsData::read(session).ok_or(CsFileError::Unsupported)?;
    confirm_link(session)?;
    let caps = session.capabilities();
    let meta = CsMeta {
        saved_utc: Some(utc_now()),
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        platform: Some(caps.platform.name()),
        firmware_version: Some(caps.firmware.clone()),
        caps_version: data.caps.caps_version,
    };
    Ok(ControlSurfaceDocument {
        kind: CS_DOCUMENT_KIND.into(),
        schema_version: CS_SCHEMA_VERSION,
        meta,
        bindings: data.bindings.iter().map(CsBindingBlock::from).collect(),
        names: data.names.clone(),
        ir: data
            .ir
            .iter()
            .map(|c| CsIrBlock {
                noun: c.noun,
                action: c.action,
                flags: c.flags,
                target: c.target,
                index: c.index,
                protocol: c.protocol,
                value: c.value,
                step: c.step,
                code: c.code,
            })
            .collect(),
        groups: data
            .groups
            .iter()
            .map(|g| CsGroupBlock {
                target_kind: g.target_kind,
                member_mask: g.member_mask,
                name: g.name.clone(),
            })
            .collect(),
        macros: data.macros.iter().map(macro_block).collect(),
        display: (data.max_pages > 0).then(|| CsDisplayBlock {
            config: display_config_block(&data.display_cfg),
            pages: data
                .pages
                .iter()
                .map(|p| CsPageBlock {
                    noun: p.noun,
                    target: p.target,
                    index: p.index,
                    flags: p.flags,
                })
                .collect(),
        }),
    })
}

fn confirm_link(session: &mut Session) -> Result<(), ApplyError> {
    let limits = session
        .capabilities()
        .cs
        .as_ref()
        .map(Limits::from)
        .unwrap_or_default();
    session
        .with_transport(|t| Ok(Surfaces::new(t, limits).read_status()))?
        .map(|_| ())
        .map_err(ApplyError::from)
}

fn macro_block(m: &CsMacro) -> CsMacroBlock {
    CsMacroBlock {
        name: m.name.clone(),
        steps: m
            .active_steps()
            .iter()
            .map(|s| CsStepBlock {
                noun: s.noun,
                action: s.action,
                flags: s.flags,
                target: s.target,
                index: s.index,
                value: s.value,
                step: s.step,
                pre_delay: s.pre_delay,
            })
            .collect(),
    }
}

fn display_config_block(c: &CsDisplayCfg) -> CsDisplayConfigBlock {
    CsDisplayConfigBlock {
        mode: c.mode,
        home_page: c.home_page,
        dwell: c.dwell,
        overlay_hold: c.overlay_hold,
        brightness: c.brightness,
        flags: c.flags,
        edit_timeout: c.edit_timeout,
    }
}

/// What a control-surface import did. Only what differs from the device is
/// written, so "unchanged" records cost no flash wear or SETs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsApplyReport {
    pub bindings: usize,
    pub names: usize,
    pub ir: usize,
    pub groups: usize,
    pub macros: usize,
    pub display_config: bool,
    pub pages: usize,
    /// Records the device refused, each with the device's reason.
    pub refused: Vec<String>,
    /// Records the device has no room for.
    pub beyond: Vec<String>,
    /// The link failed: the import stopped here, part applied.
    pub failed: Option<String>,
}

impl CsApplyReport {
    /// Whether the device took everything the document asked for.
    pub fn is_clean(&self) -> bool {
        self.refused.is_empty() && self.beyond.is_empty() && self.failed.is_none()
    }

    /// The report for the user. `saved` is whether the result went to flash.
    pub fn lines(&self, dry_run: bool, saved: bool) -> Vec<String> {
        let mut lines = vec![format!(
            "{} {} bindings, {} names, {} IR commands, {} groups, {} macros, {} display pages{}.",
            if dry_run { "Would change" } else { "Changed" },
            self.bindings,
            self.names,
            self.ir,
            self.groups,
            self.macros,
            self.pages,
            if self.display_config {
                ", the display configuration"
            } else {
                ""
            }
        )];
        lines.extend(
            self.beyond
                .iter()
                .map(|b| format!("Not on this device: {b}")),
        );
        lines.extend(self.refused.iter().map(|r| format!("Refused: {r}")));
        if let Some(f) = &self.failed {
            lines.push(format!("Stopped: {f}"));
        }
        if !dry_run {
            lines.push(if saved {
                "Saved to the device.".into()
            } else {
                "These changes are live but not stored on the device: `--save` stores them.".into()
            });
        }
        lines
    }
}

/// Apply a control-surface document, writing only what differs.
///
/// Order matters on the device: groups first, since a binding or macro may
/// name one; then every binding that changes is cleared before any is set, so
/// that two bindings exchanging pins do not refuse each other (status `0x1A`,
/// and `0x02`). A slot the document does not list is left alone. Honours
/// `session.dry_run`: the device is read and nothing is written.
///
/// # Errors
/// [`CsFileError::Unsupported`] on firmware without control surfaces. A refused
/// record or a failing link is in the report, with what got applied before it.
pub fn apply_control_surfaces(
    session: &mut Session,
    doc: &ControlSurfaceDocument,
) -> Result<CsApplyReport, CsFileError> {
    let now = CsData::read(session).ok_or(CsFileError::Unsupported)?;
    confirm_link(session)?;
    let dry = session.dry_run;
    let mut report = CsApplyReport::default();

    // One write, `true` when the import may go on. Every outcome lands in the
    // report; the link failing is the only thing that stops it.
    fn send(
        session: &mut Session,
        dry: bool,
        report: &mut CsApplyReport,
        what: String,
        f: impl FnOnce(&mut Surfaces<'_>) -> crate::surfaces::Result<CsStatusPacket>,
    ) -> Option<bool> {
        if dry {
            return Some(true);
        }
        match cs_apply(session, f) {
            Ok(()) => Some(true),
            Err(ApplyError::Rejected { status }) => {
                report
                    .refused
                    .push(format!("{what}: {}", explain_status(status)));
                Some(false)
            }
            Err(e) => {
                report.failed = Some(format!("{what}: {e}"));
                None
            }
        }
    }

    let beyond = |report: &mut CsApplyReport, what: &str, have: usize, extra: usize| {
        if extra > 0 {
            report
                .beyond
                .push(format!("{extra} {what} (this device has {have})"));
        }
    };

    // ---- groups
    let groups = doc.groups.iter().take(now.groups.len());
    let extra = doc.groups.iter().skip(now.groups.len());
    let extra = extra.filter(|g| **g != CsGroupBlock::default()).count();
    beyond(&mut report, "groups", now.groups.len(), extra);
    for (i, (g, cur)) in groups.zip(&now.groups).enumerate() {
        let want = CsGroup {
            target_kind: g.target_kind,
            member_mask: g.member_mask,
            name: g.name.clone(),
        };
        if want != *cur {
            let what = format!("group {}", i + 1);
            match send(session, dry, &mut report, what, |s| {
                s.write_group(i as u8, &want)
            }) {
                Some(true) => report.groups += 1,
                Some(false) => {}
                None => return Ok(report),
            }
        }
    }

    // ---- bindings: clear, then set
    let wanted: Vec<CsBinding> = doc
        .bindings
        .iter()
        .take(now.bindings.len())
        .map(CsBinding::from)
        .collect();
    let extra = doc.bindings.iter().skip(now.bindings.len());
    let extra = extra.filter(|b| b.r#type != 0).count();
    beyond(&mut report, "bindings", now.bindings.len(), extra);
    for (i, (want, cur)) in wanted.iter().zip(&now.bindings).enumerate() {
        if want != cur && !cur.is_empty() {
            let what = format!("clearing binding {}", i + 1);
            match send(session, dry, &mut report, what, |s| {
                s.write_binding(i as u8, &CsBinding::default())
            }) {
                Some(true) if want.is_empty() => report.bindings += 1,
                Some(_) => {}
                None => return Ok(report),
            }
        }
    }
    for (i, (want, cur)) in wanted.iter().zip(&now.bindings).enumerate() {
        // An empty slot is empty whatever its other bytes hold.
        if want == cur || want.is_empty() {
            continue;
        }
        let what = format!("binding {}", i + 1);
        match send(session, dry, &mut report, what, |s| {
            s.write_binding(i as u8, want)
        }) {
            Some(true) => report.bindings += 1,
            Some(false) => {}
            None => return Ok(report),
        }
    }
    for (i, (name, cur)) in doc.names.iter().zip(&now.names).enumerate() {
        if name != cur {
            let what = format!("name of binding {}", i + 1);
            match send(session, dry, &mut report, what, |s| {
                s.write_name(i as u8, name)
            }) {
                Some(true) => report.names += 1,
                Some(false) => {}
                None => return Ok(report),
            }
        }
    }

    // ---- IR
    let extra = doc.ir.iter().skip(now.ir.len());
    let extra = extra.filter(|c| c.protocol != 0).count();
    beyond(&mut report, "IR commands", now.ir.len(), extra);
    for (i, (c, cur)) in doc.ir.iter().zip(&now.ir).enumerate() {
        let want = ir_command(c);
        if want != *cur {
            let what = format!("IR command {}", i + 1);
            match send(session, dry, &mut report, what, |s| {
                s.write_ir_command(i as u8, &want)
            }) {
                Some(true) => report.ir += 1,
                Some(false) => {}
                None => return Ok(report),
            }
        }
    }

    // ---- macros: padded, so steps past the new count are cleared
    let extra = doc.macros.iter().skip(now.macros.len());
    let extra = extra.filter(|m| **m != CsMacroBlock::default()).count();
    beyond(&mut report, "macros", now.macros.len(), extra);
    let max_steps = now.caps.max_macro_steps as usize;
    for (i, (m, cur)) in doc.macros.iter().zip(&now.macros).enumerate() {
        if m.steps.len() > max_steps {
            beyond(
                &mut report,
                &format!("steps of macro {}", i + 1),
                max_steps,
                m.steps.len() - max_steps,
            );
        }
        let kept = &m.steps[..m.steps.len().min(max_steps)];
        let same = m.name == cur.name && kept == macro_block(cur).steps.as_slice();
        if same {
            continue;
        }
        let mut steps: Vec<CsMacroStep> = kept.iter().map(macro_step).collect();
        let header = CsMacroHeaderWire {
            name: m.name.clone(),
            step_count: steps.len() as u8,
        };
        steps.resize(max_steps, CsMacroStep::default());
        let what = format!("macro {}", i + 1);
        match send(session, dry, &mut report, what, |s| {
            s.write_macro(i as u8, &header, &steps)
        }) {
            Some(true) => report.macros += 1,
            Some(false) => {}
            None => return Ok(report),
        }
    }

    // ---- the display
    if let Some(display) = &doc.display {
        if now.max_pages == 0 {
            report.beyond.push("the display".into());
        } else {
            if display.config != display_config_block(&now.display_cfg) {
                let cfg = display_cfg(&display.config);
                match send(session, dry, &mut report, "display".into(), |s| {
                    s.write_display_cfg(&cfg)
                }) {
                    Some(true) => report.display_config = true,
                    Some(false) => {}
                    None => return Ok(report),
                }
            }
            let extra = display.pages.iter().skip(now.pages.len());
            let extra = extra.filter(|p| **p != CsPageBlock::default()).count();
            beyond(&mut report, "display pages", now.pages.len(), extra);
            for (i, (p, cur)) in display.pages.iter().zip(&now.pages).enumerate() {
                let want = CsDisplayPage {
                    noun: p.noun,
                    target: p.target,
                    index: p.index,
                    flags: p.flags,
                };
                if want != *cur {
                    let what = format!("display page {}", i + 1);
                    match send(session, dry, &mut report, what, |s| {
                        s.write_display_page(i as u8, &want)
                    }) {
                        Some(true) => report.pages += 1,
                        Some(false) => {}
                        None => return Ok(report),
                    }
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> PresetDocument {
        PresetDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            meta: Meta {
                name: Some("Living Room".into()),
                platform: Some("RP2350".into()),
                wire_format_version: 26,
                input_channel_count: 8,
                output_channel_count: 9,
                ..Default::default()
            },
            global: GlobalBlock {
                input_preamps_db: vec![0.0; 8],
                input_pair_linked: vec![false; 4],
                ..Default::default()
            },
            loudness: LoudnessBlock::default(),
            crossfeed: CrossfeedBlock {
                output_pair_mask: Some(0x01),
                ..Default::default()
            },
            leveller: LevellerBlock::default(),
            psybass: None,
            upmix: None,
            subharm: None,
            tube: None,
            channels: vec![ChannelBlock {
                channel_id: 0,
                name: "USB 1".into(),
                is_output: false,
                eq: vec![BandBlock {
                    r#type: 2,
                    freq_hz: 105.0,
                    gain: 8.8,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            matrix: vec![CrosspointBlock {
                input: 0,
                output: 0,
                enabled: true,
                invert: false,
                gain_db: -3.0,
            }],
            io: IoBlock::default(),
        }
    }

    #[test]
    fn a_document_round_trips() {
        let original = doc();
        let parsed = parse(&write(&original)).unwrap();
        assert_eq!(parsed, original);
    }

    /// The schema is camelCase, because that is what the other applications
    /// read. Getting this wrong produces a file nothing else can open.
    #[test]
    fn the_json_uses_the_interchange_spelling() {
        let text = write(&doc());
        for key in [
            "schemaVersion",
            "inputPreampsDb",
            "channelId",
            "freqHz",
            "isOutput",
            "inputPairLinked",
            "outputPairMask",
        ] {
            assert!(text.contains(key), "missing key {key} in:\n{text}");
        }
        assert!(!text.contains("schema_version"), "snake_case leaked out");
    }

    /// A block absent means the source device lacked the feature; a block
    /// present but disabled means it had it and it was off. Collapsing the two
    /// would lose real information.
    #[test]
    fn absent_and_disabled_features_are_distinguishable() {
        let mut d = doc();
        assert!(d.psybass.is_none());
        assert!(!write(&d).contains("psybass"), "absent blocks are omitted");

        d.psybass = Some(PsybassBlock {
            enabled: Some(false),
            ..Default::default()
        });
        let text = write(&d);
        assert!(text.contains("psybass"));
        assert_eq!(parse(&text).unwrap().psybass.unwrap().enabled, Some(false));
    }

    /// Refusing a newer document beats guessing at blocks we cannot read and
    /// applying half a tuning.
    #[test]
    fn a_document_from_the_future_is_refused() {
        let mut d = doc();
        d.schema_version = CURRENT_SCHEMA_VERSION + 1;
        assert_eq!(
            parse(&write(&d)),
            Err(PresetFileError::FromTheFuture {
                got: CURRENT_SCHEMA_VERSION + 1,
                supported: CURRENT_SCHEMA_VERSION
            })
        );
    }

    #[test]
    fn malformed_input_is_refused_with_a_reason() {
        assert_eq!(parse(""), Err(PresetFileError::Empty));
        assert!(matches!(parse("{"), Err(PresetFileError::Invalid(_))));
        assert_eq!(
            parse(r#"{"schemaVersion":1,"channels":[]}"#),
            Err(PresetFileError::NoChannels)
        );
        assert!(matches!(
            parse(r#"{"schemaVersion":0,"channels":[{"channelId":0}]}"#),
            Err(PresetFileError::Invalid(_))
        ));
    }

    /// Wire values, not names: a renamed enum member must not be able to break
    /// an old file.
    #[test]
    fn filter_types_are_stored_as_wire_values() {
        let text = write(&doc());
        assert!(
            text.contains("\"type\": 2"),
            "expected a raw type in:\n{text}"
        );
        assert!(!text.to_lowercase().contains("lowshelf"));
    }

    /// A channel the device does not have is reported, never moved somewhere
    /// else; the reference implementation reports rather than translating.
    #[test]
    fn a_channel_the_device_lacks_is_reported() {
        let mut d = doc();
        d.channels.push(ChannelBlock {
            // Windows id 8: S/PDIF 4 L, output 6.
            channel_id: 8,
            name: "SPDIF 4 L".into(),
            is_output: true,
            ..Default::default()
        });

        // A two-input, five-output part: output 6 does not exist there.
        let (placed, missing) = place_channels(&d, 2, 5);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[&ChannelRef::Input(0)].channel_id, 0);
        assert_eq!(missing, vec!["SPDIF 4 L"]);
    }

    #[test]
    fn a_duplicated_channel_takes_the_last_rather_than_failing() {
        let mut d = doc();
        d.channels.push(ChannelBlock {
            channel_id: 0,
            name: "USB 1 again".into(),
            delay_ms: Some(5.0),
            ..Default::default()
        });
        let (placed, missing) = place_channels(&d, 2, 5);
        assert_eq!(placed.len(), 1);
        assert_eq!(
            placed[&ChannelRef::Input(0)].delay_ms,
            Some(5.0),
            "the last block should win"
        );
        assert!(missing.is_empty());
    }

    /// Volume and wiring describe a room and a board, not a tuning, so importing
    /// them without being asked would change how loud someone's room gets.
    #[test]
    fn only_audio_processing_is_applied_by_default() {
        let o = ApplyOptions::default();
        assert!(o.audio_processing);
        assert!(!o.volume_levels);
        assert!(!o.hardware_io);
    }

    #[test]
    fn unknown_keys_from_a_newer_writer_do_not_break_parsing() {
        let text = r#"{
            "schemaVersion": 1,
            "somethingNew": {"a": 1},
            "channels": [{"channelId": 0, "name": "In", "isOutput": false}]
        }"#;
        let d = parse(text).unwrap();
        assert_eq!(d.channels.len(), 1);
    }

    #[test]
    fn missing_optional_blocks_stay_missing() {
        let text = r#"{"schemaVersion":1,"channels":[{"channelId":0,"isOutput":false}]}"#;
        let d = parse(text).unwrap();
        assert!(d.global.input_preamps_db.is_empty());
        assert_eq!(d.global.master_volume_db, None);
        assert_eq!(d.loudness, LoudnessBlock::default());
        assert!(d.psybass.is_none());
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo, Feature};
    use dspi_proto::Platform;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, LogHandle, Reply};

    fn caps() -> Capabilities {
        Capabilities {
            serial: "TEST".into(),
            platform: Platform::Rp2350,
            firmware: "1.1.6".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 6, 0),
            build_info: None,
            wire_format: dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: (0..17)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 8,
                })
                .collect(),
            features: [
                "spdif_multi_input",
                "i2s_input_channels",
                "i2s_slave_clock",
                "adat_output",
                "adat_input",
                "dac_hardware_mute",
            ]
            .iter()
            .map(|n| Feature {
                name: (*n).into(),
                present: true,
                evidence: "test".into(),
            })
            .collect(),
            cs: None,
            siggen: None,
            active_preset: None,
        }
    }

    /// A document that moves every piece of wiring, so nothing is skipped for
    /// being already correct and the whole order is exercised.
    fn io() -> IoBlock {
        IoBlock {
            output_pins: vec![6, 7, 8, 9, 10],
            output_slot_types: vec![1, 1, 1, 1],
            i2s_bck_pin: Some(14),
            mck_enabled: Some(true),
            mck_pin: Some(13),
            mck_multiplier: Some(256),
            i2s_clock_mode: Some(1),
            i2s_clock_pin_mode: Some(1),
            i2s_bck_pin_slave: Some(26),
            spdif_rx_pins: vec![5, 20, 21],
            spdif_rx_pin4: Some(22),
            spdif_enabled_ext: Some(0b111),
            i2s_rx_pins: vec![1, 2, 3, 4],
            i2s_input_channels: Some(8),
            i2s_input_rate_hz: Some(48000),
            adat_enabled: Some(true),
            adat_pin: Some(12),
            adat_input_enabled: Some(true),
            adat_input_pin: Some(11),
            adat_input_clock_mode: Some(1),
            dac_hw_mute: Some(DacHwMuteBlock {
                enabled: true,
                active_low: true,
                pin: 11,
                hold_ms: 5,
                release_ms: 0,
            }),
        }
    }

    fn doc_with(io: IoBlock) -> PresetDocument {
        PresetDocument {
            schema_version: CURRENT_SCHEMA_VERSION,
            meta: Meta::default(),
            global: GlobalBlock::default(),
            loudness: LoudnessBlock::default(),
            crossfeed: CrossfeedBlock::default(),
            leveller: LevellerBlock::default(),
            psybass: None,
            upmix: None,
            subharm: None,
            tube: None,
            channels: vec![ChannelBlock::default()],
            matrix: Vec::new(),
            io,
        }
    }

    /// Every GET answers a value the document disagrees with, and every SET
    /// answers `PIN_CONFIG_SUCCESS`.
    fn device() -> MockTransport {
        MockTransport::new()
            .data(op::REQ_GET_OUTPUT_TYPE, vec![0])
            .data(op::REQ_SET_OUTPUT_TYPE, vec![0])
            .data(op::REQ_GET_OUTPUT_PIN, vec![0])
            .data(op::REQ_SET_OUTPUT_PIN, vec![0])
            .data(op::REQ_GET_I2S_BCK_PIN, vec![0])
            .data(op::REQ_SET_I2S_BCK_PIN, vec![0])
            .data(op::REQ_GET_MCK_ENABLE, vec![1])
            .data(op::REQ_SET_MCK_ENABLE, vec![0])
            .data(op::REQ_GET_MCK_PIN, vec![0])
            .data(op::REQ_SET_MCK_PIN, vec![0])
            .data(op::REQ_GET_MCK_MULTIPLIER, vec![0])
            .data(op::REQ_SET_MCK_MULTIPLIER, vec![0])
            // {count, enable_mask, gpio[0..3]} (config.h:456-457).
            .data(op::REQ_GET_SPDIF_INPUT_CONFIG, vec![4, 0b0001, 0, 0, 0, 0])
            .data(op::REQ_GET_SPDIF_RX_PIN, vec![0])
            .data(op::REQ_SET_SPDIF_RX_PIN, vec![0])
            .data(op::REQ_SET_SPDIF_INPUT_ENABLE, vec![0])
            .data(op::REQ_GET_I2S_RX_PIN, vec![0])
            .data(op::REQ_SET_I2S_RX_PIN, vec![0])
            .data(op::REQ_GET_I2S_INPUT_CHANNELS, vec![2])
            .data(op::REQ_SET_I2S_INPUT_CHANNELS, vec![0])
            .data(op::REQ_GET_INPUT_RATE, 44100u32.to_le_bytes().to_vec())
            .data(op::REQ_SET_INPUT_RATE, vec![0])
            // Deferred OUT writes: the confirming readback has to see the new
            // value, or a correct write looks like a silent rejection.
            .reply(
                op::REQ_GET_I2S_CLOCK_MODE,
                Reply::Sequence(vec![Reply::Data(vec![0]), Reply::Data(vec![1])]),
            )
            .data(op::REQ_SET_I2S_CLOCK_MODE, vec![0])
            .data(op::REQ_GET_I2S_CLOCK_PIN_MODE, vec![0])
            .data(op::REQ_SET_I2S_CLOCK_PIN_MODE, vec![0])
            .data(op::REQ_GET_ADAT_PIN, vec![0])
            .data(op::REQ_SET_ADAT_PIN, vec![0])
            .data(op::REQ_GET_ADAT_ENABLE, vec![0])
            .data(op::REQ_SET_ADAT_ENABLE, vec![0])
            .data(op::REQ_GET_ADAT_INPUT_PIN, vec![0])
            .data(op::REQ_SET_ADAT_INPUT_PIN, vec![0])
            .reply(
                op::REQ_GET_ADAT_INPUT_CLOCK_MODE,
                Reply::Sequence(vec![Reply::Data(vec![0]), Reply::Data(vec![1])]),
            )
            .data(op::REQ_SET_ADAT_INPUT_CLOCK_MODE, vec![0])
            .reply(
                op::REQ_GET_ADAT_INPUT_ENABLE,
                Reply::Sequence(vec![Reply::Data(vec![0]), Reply::Data(vec![1])]),
            )
            .data(op::REQ_SET_ADAT_INPUT_ENABLE, vec![0])
            .data(op::REQ_GET_DAC_HW_MUTE_CONFIG, {
                let mut d = vec![0u8; 16];
                d[0] = 1;
                d[1] = 1;
                d[2] = 11;
                d[4] = 5;
                d
            })
            .data(op::REQ_SET_DAC_HW_MUTE_CONFIG, vec![0])
            .data(op::REQ_GET_INPUT_SOURCE, vec![0])
            .data(op::REQ_SET_INPUT_SOURCE, vec![0])
            .data(op::REQ_GET_OUTPUT_ENABLE, vec![1])
            .data(op::REQ_SET_OUTPUT_ENABLE, vec![0])
            .data(op::REQ_GET_CORE1_CONFLICT, vec![0])
    }

    fn rig(t: MockTransport) -> (Session, LogHandle) {
        let log = t.log_handle();
        (Session::new(Box::new(t), caps()).unwrap(), log)
    }

    const SETTERS: &[u8] = &[
        op::REQ_SET_OUTPUT_TYPE,
        op::REQ_SET_OUTPUT_PIN,
        op::REQ_SET_I2S_BCK_PIN,
        op::REQ_SET_MCK_ENABLE,
        op::REQ_SET_MCK_PIN,
        op::REQ_SET_MCK_MULTIPLIER,
        op::REQ_SET_SPDIF_RX_PIN,
        op::REQ_SET_SPDIF_INPUT_ENABLE,
        op::REQ_SET_I2S_INPUT_CHANNELS,
        op::REQ_SET_I2S_RX_PIN,
        op::REQ_SET_INPUT_RATE,
        op::REQ_SET_I2S_CLOCK_MODE,
        op::REQ_SET_I2S_CLOCK_PIN_MODE,
        op::REQ_SET_ADAT_PIN,
        op::REQ_SET_ADAT_ENABLE,
        op::REQ_SET_ADAT_INPUT_PIN,
        op::REQ_SET_ADAT_INPUT_CLOCK_MODE,
        op::REQ_SET_ADAT_INPUT_ENABLE,
        op::REQ_SET_DAC_HW_MUTE_CONFIG,
    ];

    /// The opcodes that changed something, in the order they went out.
    fn sets(log: &LogHandle) -> Vec<u8> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| SETTERS.contains(&e.opcode))
            .map(|e| e.opcode)
            .collect()
    }

    /// The order is the whole of the difficulty: these settings validate
    /// against each other, so a rearrangement fails silently on hardware and
    /// nowhere else.
    #[test]
    fn the_apply_order_follows_the_reference_implementation() {
        let (mut s, log) = rig(device());
        let mut report = ApplyReport::default();
        apply_io(&mut s, &io(), &mut report);

        let seen = sets(&log);
        let first = |opcode: u8| {
            seen.iter()
                .position(|o| *o == opcode)
                .unwrap_or_else(|| panic!("0x{opcode:02X} never went out: {seen:02X?}"))
        };

        // Slot types before pins: a type change is deferred and reallocates.
        assert!(first(op::REQ_SET_OUTPUT_TYPE) < first(op::REQ_SET_OUTPUT_PIN));
        // The pins before the clocks that validate against them.
        assert!(first(op::REQ_SET_OUTPUT_PIN) < first(op::REQ_SET_I2S_BCK_PIN));
        // MCK off before its pin moves, which is where this departs from the
        // reference implementation: the firmware answers OUTPUT_ACTIVE for a
        // pin change while MCK runs (survey-firmware 6.7).
        assert!(first(op::REQ_SET_MCK_ENABLE) < first(op::REQ_SET_MCK_PIN));
        assert!(first(op::REQ_SET_I2S_BCK_PIN) < first(op::REQ_SET_SPDIF_RX_PIN));
        // Each S/PDIF pin lands before the enable that validates against it.
        assert!(first(op::REQ_SET_SPDIF_RX_PIN) < first(op::REQ_SET_SPDIF_INPUT_ENABLE));
        // The channel count first: lowering it frees pairs and never fails.
        assert!(first(op::REQ_SET_I2S_INPUT_CHANNELS) < first(op::REQ_SET_I2S_RX_PIN));
        assert!(first(op::REQ_SET_I2S_RX_PIN) < first(op::REQ_SET_INPUT_RATE));
        // The slave pair is stored before SPLIT starts using it.
        assert!(first(op::REQ_SET_I2S_CLOCK_PIN_MODE) > first(op::REQ_SET_I2S_CLOCK_MODE));
        // ADAT out re-routes under a muted restart, so the pin precedes the
        // enable; the same for the input.
        assert!(first(op::REQ_SET_ADAT_PIN) < first(op::REQ_SET_ADAT_ENABLE));
        assert!(first(op::REQ_SET_ADAT_INPUT_PIN) < first(op::REQ_SET_ADAT_INPUT_ENABLE));
        assert!(first(op::REQ_SET_ADAT_INPUT_CLOCK_MODE) < first(op::REQ_SET_ADAT_INPUT_ENABLE));
        assert!(first(op::REQ_SET_ADAT_INPUT_ENABLE) < first(op::REQ_SET_DAC_HW_MUTE_CONFIG));

        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    /// The input source picks which wiring the device listens to, so it lands
    /// after the wiring has moved.
    #[test]
    fn the_input_source_is_written_after_the_wiring() {
        // The audio pass touches every feature block, and a stalled read costs
        // the retry backoff; this test is about the order, not the gating.
        let (mut s, log) = rig(device().answering_everything(vec![0; 4]));
        let mut doc = doc_with(io());
        doc.global.input_source = Some(0);
        apply(
            &mut s,
            &doc,
            ApplyOptions {
                audio_processing: true,
                volume_levels: false,
                hardware_io: true,
            },
        );

        let seen: Vec<u8> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_INPUT_SOURCE || e.opcode == op::REQ_SET_ADAT_PIN)
            .map(|e| e.opcode)
            .collect();
        assert_eq!(
            seen.last(),
            Some(&op::REQ_SET_INPUT_SOURCE),
            "the source goes last: {seen:02X?}"
        );
    }

    /// The slave BCK pin carries its role in the SET's wValue high byte, which
    /// no registry row expresses (config.h:334-336).
    #[test]
    fn the_slave_bck_pin_is_addressed_by_role() {
        let (mut s, log) = rig(device());
        apply_io(&mut s, &io(), &mut ApplyReport::default());

        let slave = log
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.opcode == op::REQ_SET_I2S_BCK_PIN && e.value >> 8 == 1)
            .cloned();
        assert_eq!(slave.map(|e| e.value & 0xFF), Some(26));
    }

    /// A refused pin is named with the reason and the apply carries on: a board
    /// that cannot take one pin can usually still take the other twenty.
    #[test]
    fn a_refused_pin_is_reported_and_the_rest_still_applies() {
        let t = device().data(
            op::REQ_SET_OUTPUT_PIN,
            vec![status::PIN_CONFIG_PIN_IN_USE as u8],
        );
        let (mut s, log) = rig(t);
        let mut report = ApplyReport::default();
        apply_io(&mut s, &io(), &mut report);

        assert!(
            report.skipped.iter().any(|x| x
                == "Output 1 GPIO rejected by the device: A pin is already claimed by another \
                    output or interface"),
            "{:?}",
            report.skipped
        );
        // One line per output, not one per attempt.
        assert_eq!(
            report.skipped.iter().filter(|x| x.contains("GPIO")).count(),
            5,
            "{:?}",
            report.skipped
        );
        // And the steps after it still went out.
        assert!(sets(&log).contains(&op::REQ_SET_ADAT_ENABLE));
    }

    /// The PDM pin cannot move while PDM runs (survey-firmware 6.7), so the
    /// output is cycled around the move the way the Console does.
    #[test]
    fn a_busy_pdm_pin_is_cycled_rather_than_given_up_on() {
        let t = device().data(
            op::REQ_SET_OUTPUT_PIN,
            vec![status::PIN_CONFIG_OUTPUT_ACTIVE as u8],
        );
        let (mut s, log) = rig(t);
        apply_io(&mut s, &io(), &mut ApplyReport::default());

        let enables: Vec<(u16, u8)> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_OUTPUT_ENABLE && e.direction == Direction::Out)
            .map(|e| (e.value, e.payload[0]))
            .collect();
        // Off, the retry, then back to what it was: only the PDM output, and
        // only for the pin the firmware called busy.
        assert_eq!(enables, vec![(8, 0), (8, 1)]);
    }

    /// An accepted pin move must not disturb the output at all.
    #[test]
    fn an_accepted_pin_move_leaves_the_output_alone() {
        let (mut s, log) = rig(device());
        apply_io(&mut s, &io(), &mut ApplyReport::default());
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_SET_OUTPUT_ENABLE && e.direction == Direction::Out)
        );
    }

    /// Nothing in the wiring block is written when the import did not ask for
    /// it, whatever the document says.
    #[test]
    fn wiring_is_left_alone_unless_it_was_asked_for() {
        let (mut s, log) = rig(device());
        let report = apply(
            &mut s,
            &doc_with(io()),
            ApplyOptions {
                audio_processing: false,
                volume_levels: false,
                hardware_io: false,
            },
        );

        assert!(!sets(&log).contains(&op::REQ_SET_OUTPUT_PIN));
        assert!(
            report
                .skipped
                .contains(&"hardware I/O (not requested)".to_string())
        );
    }

    /// An absent key means "the document does not say", not "GPIO 0". That is
    /// the difference between a truncated file being harmless and it moving
    /// the bit clock onto pin 0.
    #[test]
    fn an_absent_field_moves_nothing() {
        let (mut s, log) = rig(device());
        apply_io(&mut s, &IoBlock::default(), &mut ApplyReport::default());
        assert!(sets(&log).is_empty(), "{:02X?}", sets(&log));
    }

    /// A value already in place is not rewritten, because several of these
    /// answer OUTPUT_ACTIVE for a no-op change while the peripheral runs.
    #[test]
    fn a_setting_already_in_place_is_not_rewritten() {
        let (mut s, log) = rig(device());
        let io = IoBlock {
            // The device answers 0 for every pin read.
            i2s_bck_pin: Some(0),
            adat_pin: Some(0),
            ..Default::default()
        };
        apply_io(&mut s, &io, &mut ApplyReport::default());
        assert!(sets(&log).is_empty(), "{:02X?}", sets(&log));
    }

    /// A feature this device does not have is reported, not attempted.
    #[test]
    fn a_missing_feature_is_named_the_way_the_console_names_it() {
        let mut caps = caps();
        caps.features.clear();
        let t = device();
        let log = t.log_handle();
        let mut s = Session::new(Box::new(t), caps).unwrap();

        let mut report = ApplyReport::default();
        apply_io(&mut s, &io(), &mut report);

        for expected in [
            "Additional S/PDIF inputs (not supported by this firmware)",
            "ADAT output (not supported by this device)",
            "ADAT input (not supported by this device)",
            "External DAC hardware mute (not supported by this firmware)",
        ] {
            assert!(
                report.skipped.iter().any(|s| s == expected),
                "missing `{expected}` in {:?}",
                report.skipped
            );
        }
        assert!(!sets(&log).contains(&op::REQ_SET_ADAT_PIN));
    }

    /// The result report is the Console's, word for word.
    #[test]
    fn the_report_reads_the_way_the_console_words_it() {
        let report = ApplyReport {
            channels_applied: 9,
            bands_applied: 90,
            crossover_bands_applied: 4,
            crosspoints_applied: 72,
            missing_channels: vec!["SPDIF 3 L".into()],
            skipped: vec!["ADAT output (not supported by this device)".into()],
            ..Default::default()
        };
        assert_eq!(
            report.lines(false),
            vec![
                "Applied 9 channels, 90 EQ bands, 4 crossover bands, 72 crosspoints.",
                "Not present on this device: SPDIF 3 L",
                "Skipped: ADAT output (not supported by this device)",
                "These changes are live but not yet stored on the device. \
                 Save them to a preset slot to keep them.",
            ]
        );
        assert!(!report.is_clean());

        // A dry run promises nothing about what is live, because nothing is.
        let dry = report.lines(true);
        assert!(dry[0].starts_with("Would apply"));
        assert!(!dry.iter().any(|l| l.contains("live")));
    }

    /// A reason is recorded once, or a per-band problem buries the report.
    #[test]
    fn a_reason_is_recorded_once() {
        let mut report = ApplyReport::default();
        report.skip("ADAT output (not supported by this device)");
        report.skip("ADAT output (not supported by this device)");
        assert_eq!(report.skipped.len(), 1);
    }

    /// The document holds the multiple; the wire holds a selector.
    #[test]
    fn the_master_clock_multiple_is_not_the_wire_value() {
        assert_eq!(mck_selector(128), 0);
        assert_eq!(mck_selector(256), 1);
        assert_eq!(mck_multiple(0), 128);
        assert_eq!(mck_multiple(1), 256);
    }

    /// Captured wiring has to survive a round trip through the file, or an
    /// export followed by an import with `--hardware` would move pins that
    /// were never meant to move.
    #[test]
    fn the_wiring_round_trips_through_the_document() {
        let (mut s, _) = rig(device());
        let captured = capture_io(&mut Reader::new(&mut s)).unwrap();
        assert_eq!(captured.output_pins.len(), 5, "four slots and the PDM sub");
        assert_eq!(captured.i2s_bck_pin, Some(0));
        assert_eq!(captured.spdif_enabled_ext, Some(0));
        assert_eq!(captured.dac_hw_mute.as_ref().map(|m| m.pin), Some(11));
        // The slave BCK pin has no scalar reader, so it stays absent rather
        // than being invented.
        assert_eq!(captured.i2s_bck_pin_slave, None);

        let doc = doc_with(captured.clone());
        let text = write(&doc);
        assert_eq!(parse(&text).unwrap().io, captured);
        assert!(!text.contains("i2sBckPinSlave"), "absent keys are omitted");
    }
}

/// The beta4 blocks: subharm and tube at the top level, and a limiter inside
/// each output's channel entry (PresetDocument.swift:45-47, 235-432).
#[cfg(test)]
mod beta4_tests {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo, Feature};
    use dspi_proto::Platform;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, LogHandle};

    fn caps(features: &[&str]) -> Capabilities {
        Capabilities {
            serial: "TEST".into(),
            platform: Platform::Rp2350,
            firmware: "1.1.6 beta 4".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 6, 4),
            build_info: None,
            wire_format: dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8,
            num_channels: 17,
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
            band_storage: 12,
            channels: (0..17)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 8,
                })
                .collect(),
            features: features
                .iter()
                .map(|n| Feature {
                    name: (*n).into(),
                    present: true,
                    evidence: "test".into(),
                })
                .collect(),
            cs: None,
            siggen: None,
            active_preset: None,
        }
    }

    const ALL: &[&str] = &["subharmonic_synth", "tube_preamp", "output_limiter"];

    /// A device that answers everything, so the apply's readbacks never wait
    /// out a stall.
    fn rig(features: &[&str]) -> (Session, LogHandle) {
        // The canned 64 bytes are no bulk image, and a short one fails a capture.
        let t = MockTransport::new()
            .reply(
                op::REQ_GET_ALL_PARAMS_CHUNK,
                dspi_transport::mock::Reply::Stall,
            )
            .answering_everything(vec![0; 64]);
        let log = t.log_handle();
        (Session::new(Box::new(t), caps(features)).unwrap(), log)
    }

    /// `(opcode, wValue, payload)` of every write, in order.
    fn writes(log: &LogHandle, opcodes: &[u8]) -> Vec<(u8, u16, Vec<u8>)> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && opcodes.contains(&e.opcode))
            .map(|e| (e.opcode, e.value, e.payload.clone()))
            .collect()
    }

    fn doc() -> PresetDocument {
        let mut d = parse(r#"{"schemaVersion":1,"channels":[{"channelId":0}]}"#).unwrap();
        d.subharm = Some(SubharmBlock {
            enabled: Some(true),
            low_db: Some(-6.0),
            high_db: Some(-6.0),
            top_db: Some(0.0),
            boost_db: Some(3.0),
            output_mask: Some(0x0100),
            select_mode: Some(1),
            select_depth_pct: Some(80.0),
            select_hold_ms: Some(200.0),
            ceiling_db: Some(-10.0),
            link_pairs: Some(false),
        });
        d.tube = Some(TubeBlock {
            enabled: Some(true),
            output_mask: Some(0xFFFF),
            tube_type: Some(0),
            drive_db: Some(-12.0),
            bias_pct: Some(20.0),
            asym_db: Some(0.0),
            hardness_pct: Some(50.0),
            sag_pct: Some(0.0),
            rectifier: Some(0),
            xfmr_enabled: Some(true),
            xfmr_damping: Some(4.0),
            xfmr_res_hz: Some(95.0),
            mix_pct: Some(100.0),
            trim_db: Some(0.0),
        });
        d.channels.push(ChannelBlock {
            channel_id: channel_id::for_output(0),
            name: "Main L".into(),
            is_output: true,
            limiter: Some(LimiterBlock {
                enabled: Some(true),
                threshold_db: Some(-3.0),
                release_ms: Some(200.0),
                link_group: Some(1),
            }),
            ..Default::default()
        });
        d
    }

    #[test]
    fn the_new_blocks_round_trip() {
        let original = doc();
        let text = write(&original);
        assert_eq!(parse(&text).unwrap(), original);
    }

    /// Exactly the Console's key names; anything else is a file the Console
    /// reads as defaults.
    #[test]
    fn the_new_blocks_use_the_consoles_keys() {
        let text = write(&doc());
        for key in [
            "\"subharm\"",
            "\"lowDb\"",
            "\"highDb\"",
            "\"topDb\"",
            "\"boostDb\"",
            "\"selectMode\"",
            "\"selectDepthPct\"",
            "\"selectHoldMs\"",
            "\"ceilingDb\"",
            "\"linkPairs\"",
            "\"tube\"",
            "\"tubeType\"",
            "\"driveDb\"",
            "\"biasPct\"",
            "\"asymDb\"",
            "\"hardnessPct\"",
            "\"sagPct\"",
            "\"rectifier\"",
            "\"xfmrEnabled\"",
            "\"xfmrDamping\"",
            "\"xfmrResHz\"",
            "\"mixPct\"",
            "\"trimDb\"",
            "\"limiter\"",
            "\"thresholdDb\"",
            "\"releaseMs\"",
            "\"linkGroup\"",
        ] {
            assert!(text.contains(key), "missing {key} in:\n{text}");
        }
        assert!(!text.contains("outputIndex"), "this build writes none");
        // Masks and enums are raw numbers (PresetDocument.swift:245-246, 279).
        assert!(text.contains("\"outputMask\": 256"), "{text}");
        assert!(text.contains("\"selectMode\": 1"), "{text}");
    }

    /// Absent means the source device lacked the feature, and an input never
    /// carries a limiter.
    #[test]
    fn absent_blocks_stay_absent() {
        let d = parse(r#"{"schemaVersion":1,"channels":[{"channelId":0}]}"#).unwrap();
        assert!(d.subharm.is_none() && d.tube.is_none());
        assert!(d.channels[0].limiter.is_none());
        let text = write(&d);
        assert!(!text.contains("subharm") && !text.contains("tube"));
        assert!(!text.contains("limiter"));
    }

    /// A document as the Console writes one: JSONEncoder, pretty printed with
    /// sorted keys (PresetDocument.swift:705-711), from `capture`
    /// (PresetDocumentTransfer.swift:20-163), on an RP2350 at wire V32. The
    /// shape was built by hand from the Swift encoder, since no exported
    /// sample is checked in.
    const CONSOLE_DOCUMENT: &str = r#"{
  "channels" : [
    {
      "channelId" : 0,
      "crossover" : [

      ],
      "delayMs" : 0,
      "enabled" : true,
      "eq" : [
        {
          "bypass" : false,
          "freqHz" : 105,
          "gain" : 6.5,
          "q" : 0.707,
          "qp" : 0.707,
          "type" : 2
        }
      ],
      "eqChannel" : 0,
      "gainDb" : 0,
      "inputIndex" : 0,
      "isOutput" : false,
      "muted" : false,
      "name" : "USB L"
    },
    {
      "channelId" : 3,
      "crossover" : [

      ],
      "delayMs" : 0,
      "enabled" : true,
      "eq" : [

      ],
      "eqChannel" : 9,
      "gainDb" : -3,
      "isOutput" : true,
      "limiter" : {
        "enabled" : true,
        "linkGroup" : 2,
        "releaseMs" : 250,
        "thresholdDb" : -4.5
      },
      "muted" : false,
      "name" : "SPDIF 1 R",
      "outputDelayMs" : 1.5,
      "outputIndex" : 1
    }
  ],
  "crossfeed" : {
    "enabled" : false,
    "feedDb" : 4.5,
    "freqHz" : 700,
    "itd" : true,
    "outputPairMask" : 1,
    "preset" : 0
  },
  "global" : {
    "bypass" : false,
    "inputPairLinked" : [
      false,
      false,
      false,
      false
    ],
    "inputPreampsDb" : [
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0
    ],
    "inputSource" : 0,
    "lgSoundSyncEnabled" : false,
    "masterVolumeDb" : -20,
    "userVolumeDb" : -12
  },
  "io" : {
    "adatEnabled" : false,
    "adatInputClockMode" : 0,
    "adatInputEnabled" : false,
    "adatInputPin" : 255,
    "adatPin" : 12,
    "i2sBckPin" : 14,
    "i2sBckPinSlave" : 26,
    "i2sClockMode" : 0,
    "i2sClockPinMode" : 0,
    "i2sInputChannels" : 2,
    "i2sInputRateHz" : 48000,
    "i2sRxPins" : [
      1,
      2,
      3,
      4
    ],
    "mckEnabled" : false,
    "mckMultiplier" : 128,
    "mckPin" : 13,
    "outputPins" : [
      6,
      7,
      8,
      9,
      10
    ],
    "outputSlotTypes" : [
      0,
      0,
      0,
      0
    ],
    "spdifEnabledExt" : 0,
    "spdifRxPins" : [
      5,
      20,
      21
    ]
  },
  "leveller" : {
    "amountPct" : 50,
    "applyMask" : 255,
    "detectorMask" : 255,
    "enabled" : false,
    "gateDb" : -96,
    "lookahead" : true,
    "maxGainDb" : 15,
    "speed" : 0
  },
  "loudness" : {
    "enabled" : false,
    "intensityPct" : 100,
    "outputMask" : 65535,
    "refSpl" : 83
  },
  "matrix" : [
    {
      "enabled" : true,
      "gainDb" : 0,
      "input" : 0,
      "invert" : false,
      "output" : 0
    }
  ],
  "meta" : {
    "appVersion" : "1.1.6-beta4",
    "firmwareVersion" : "1.1.6 beta 4",
    "inputChannelCount" : 8,
    "masterVolumeMode" : 0,
    "name" : "Living Room",
    "outputChannelCount" : 9,
    "outputConfigMode" : 1,
    "platform" : "RP2350",
    "savedUtc" : "2026-09-29T10:00:00Z",
    "wireFormatVersion" : 32
  },
  "psybass" : {
    "characterPct" : 50,
    "cutoffHz" : 80,
    "driveDb" : 6,
    "enabled" : false,
    "harmonicsDb" : 0,
    "originalDb" : 0,
    "outputMask" : 65535
  },
  "schemaVersion" : 1,
  "subharm" : {
    "boostDb" : 3,
    "ceilingDb" : -12,
    "enabled" : true,
    "highDb" : -6,
    "linkPairs" : true,
    "lowDb" : -6,
    "outputMask" : 256,
    "selectDepthPct" : 100,
    "selectHoldMs" : 150,
    "selectMode" : 2,
    "topDb" : -30
  },
  "tube" : {
    "asymDb" : 2,
    "biasPct" : 5,
    "driveDb" : -6,
    "enabled" : true,
    "hardnessPct" : 55,
    "mixPct" : 80,
    "outputMask" : 255,
    "rectifier" : 2,
    "sagPct" : 10,
    "trimDb" : -1.5,
    "tubeType" : 3,
    "xfmrDamping" : 4,
    "xfmrEnabled" : false,
    "xfmrResHz" : 80
  },
  "upmix" : {
    "attackMs" : 10,
    "centerMode" : 1,
    "centerWidthPct" : 25,
    "decorrPct" : 90,
    "detectorHpfHz" : 200,
    "enabled" : false,
    "presenceDb" : 0,
    "releaseMs" : 100,
    "strengthPct" : 100,
    "surroundDelayMs" : 12,
    "surroundHpfHz" : 300,
    "surroundLpfHz" : 7000,
    "surroundMode" : 1,
    "thresholdPct" : 30
  }
}"#;

    #[test]
    fn a_document_in_the_consoles_format_parses() {
        let d = parse(CONSOLE_DOCUMENT).unwrap();
        assert_eq!(d.meta.wire_format_version, 32);
        assert_eq!(
            d.subharm,
            Some(SubharmBlock {
                enabled: Some(true),
                low_db: Some(-6.0),
                high_db: Some(-6.0),
                top_db: Some(-30.0),
                boost_db: Some(3.0),
                output_mask: Some(0x0100),
                select_mode: Some(2),
                select_depth_pct: Some(100.0),
                select_hold_ms: Some(150.0),
                ceiling_db: Some(-12.0),
                link_pairs: Some(true),
            })
        );
        assert_eq!(
            d.tube,
            Some(TubeBlock {
                enabled: Some(true),
                output_mask: Some(0xFF),
                tube_type: Some(3),
                drive_db: Some(-6.0),
                bias_pct: Some(5.0),
                asym_db: Some(2.0),
                hardness_pct: Some(55.0),
                sag_pct: Some(10.0),
                rectifier: Some(2),
                xfmr_enabled: Some(false),
                xfmr_damping: Some(4.0),
                xfmr_res_hz: Some(80.0),
                mix_pct: Some(80.0),
                trim_db: Some(-1.5),
            })
        );
        assert!(d.channels[0].limiter.is_none(), "inputs carry none");
        let out = &d.channels[1];
        assert_eq!(out.output_index, Some(1));
        assert_eq!(
            out.limiter,
            Some(LimiterBlock {
                enabled: Some(true),
                threshold_db: Some(-4.5),
                release_ms: Some(250.0),
                link_group: Some(2),
            })
        );
    }

    /// A Console before V30 wrote a two-band subharm block, and a hand-edited
    /// file may drop any key: what is missing stays missing, and an import leaves
    /// it alone (WP TM21). It used to take the firmware's defaults, as the
    /// Console's lenient decoder gives them (PresetDocument.swift:252-266).
    #[test]
    fn a_partial_block_holds_only_its_own_keys() {
        let d = parse(
            r#"{"schemaVersion":1,
                "subharm":{"enabled":true,"lowDb":-3,"highDb":0,"boostDb":2,"outputMask":256},
                "tube":{"enabled":true},
                "channels":[{"channelId":8,"isOutput":true,"limiter":{"enabled":true}}]}"#,
        )
        .unwrap();
        let s = d.subharm.as_ref().unwrap();
        assert_eq!(s.low_db, Some(-3.0));
        assert_eq!(s.top_db, None);
        assert_eq!(s.link_pairs, None);
        let tube = TubeBlock {
            enabled: Some(true),
            ..Default::default()
        };
        assert_eq!(d.tube.as_ref(), Some(&tube));
        let l = d.channels[0].limiter.as_ref().unwrap();
        assert_eq!(
            (l.enabled, l.threshold_db, l.release_ms, l.link_group),
            (Some(true), None, None, None)
        );
        let left = omitted_settings(&d);
        assert!(left.contains(&"subharm.topDb".to_string()), "{left:?}");
        assert!(left.contains(&"tube.driveDb".to_string()), "{left:?}");
        assert!(
            left.contains(&"channels.limiter.thresholdDb".to_string()),
            "{left:?}"
        );
    }

    /// Every value first and the switch last, and the tube type before the
    /// four values it loads (PresetDocumentTransfer.swift:600-644).
    #[test]
    fn subharm_and_tube_apply_in_the_consoles_order() {
        let (mut s, log) = rig(ALL);
        let report = apply(&mut s, &doc(), ApplyOptions::default());
        let subharm = [
            op::REQ_SET_SUBHARM_LOW,
            op::REQ_SET_SUBHARM_HIGH,
            op::REQ_SET_SUBHARM_BOOST,
            op::REQ_SET_SUBHARM_MASK,
            op::REQ_SET_SUBHARM_TOP,
            op::REQ_SET_SUBHARM_SELECT,
            op::REQ_SET_SUBHARM_DEPTH,
            op::REQ_SET_SUBHARM_HOLD,
            op::REQ_SET_SUBHARM_CEILING,
            op::REQ_SET_SUBHARM_LINK,
            op::REQ_SET_SUBHARM,
        ];
        let sent: Vec<u8> = writes(&log, &subharm).iter().map(|w| w.0).collect();
        assert_eq!(sent, subharm.to_vec());
        let tube: Vec<u16> = writes(&log, &[op::REQ_SET_TUBE_PARAM])
            .iter()
            .map(|w| w.1)
            .collect();
        // TUBE_PARAM_* (tube.h:17-30): type, bias, asym, hardness, sag,
        // drive, rectifier, damping, resonance, output stage, mix, trim,
        // mask, enable.
        assert_eq!(tube, vec![2, 4, 5, 6, 7, 3, 8, 10, 11, 9, 12, 13, 1, 0]);
        // Every tube value is a float on the wire, the mask included.
        let mask = writes(&log, &[op::REQ_SET_TUBE_PARAM])[12].2.clone();
        assert_eq!(mask, 65535.0f32.to_le_bytes().to_vec());
        assert!(
            !report.skipped.iter().any(|r| r.contains("not supported")),
            "{:?}",
            report.skipped
        );
    }

    /// The Console's skip reasons (PresetDocumentTransfer.swift:435, 618,
    /// 643), and nothing written for a feature the device lacks.
    #[test]
    fn a_missing_feature_is_skipped_with_the_consoles_reason() {
        let (mut s, log) = rig(&[]);
        let report = apply(
            &mut s,
            &doc(),
            ApplyOptions {
                hardware_io: true,
                ..Default::default()
            },
        );
        for reason in [
            "Subharmonic synthesizer (not supported by this firmware)",
            "Tube preamp (not supported by this firmware)",
            "Output limiter (not supported by this firmware)",
        ] {
            assert!(
                report.skipped.iter().any(|r| r == reason),
                "{reason} not in {:?}",
                report.skipped
            );
        }
        let touched = writes(
            &log,
            &[op::REQ_SET_SUBHARM, op::REQ_SET_TUBE_PARAM, op::REQ_LIMITER],
        );
        assert!(touched.is_empty(), "{touched:?}");
    }

    /// Limiters are output configuration: only the hardware I/O option brings
    /// them in (PresetDocumentTransfer.swift:246-250, 323-326).
    #[test]
    fn limiters_apply_only_with_the_hardware_option() {
        let (mut s, log) = rig(ALL);
        apply(&mut s, &doc(), ApplyOptions::default());
        assert!(writes(&log, &[op::REQ_LIMITER]).is_empty());

        let (mut s, log) = rig(ALL);
        apply(
            &mut s,
            &doc(),
            ApplyOptions {
                hardware_io: true,
                ..Default::default()
            },
        );
        let w = writes(&log, &[op::REQ_LIMITER]);
        // Output 0 (Windows id 2): threshold, release, enable, then the group.
        let values: Vec<u16> = w.iter().map(|w| w.1).collect();
        assert_eq!(values, vec![0x0001, 0x0002, 0x0000, 0x0003]);
        assert_eq!(w[0].2, (-3.0f32).to_le_bytes().to_vec());
        assert_eq!(w[3].2, 1.0f32.to_le_bytes().to_vec());
    }

    /// A linked output is unlinked before its values go out, so one write
    /// does not move its whole group (limiter.c:143-194), and relinked last.
    /// The Console's own output index places the block.
    #[test]
    fn a_linked_output_is_unlinked_first_and_relinked_last() {
        let t = MockTransport::new()
            .answering_everything(vec![0; 64])
            // Every output reads as linked to group 2.
            .data(op::REQ_LIMITER, 2.0f32.to_le_bytes().to_vec());
        let log = t.log_handle();
        let mut s = Session::new(Box::new(t), caps(ALL)).unwrap();
        let d = parse(CONSOLE_DOCUMENT).unwrap();
        apply(
            &mut s,
            &d,
            ApplyOptions {
                hardware_io: true,
                ..Default::default()
            },
        );
        let w = writes(&log, &[op::REQ_LIMITER]);
        let values: Vec<u16> = w.iter().map(|w| w.1).collect();
        // Output 1, from `outputIndex` rather than `channelId` 3.
        assert_eq!(values, vec![0x0103, 0x0101, 0x0102, 0x0100, 0x0103]);
        assert_eq!(w[0].2, 0.0f32.to_le_bytes().to_vec(), "unlinked");
        assert_eq!(w[4].2, 2.0f32.to_le_bytes().to_vec(), "relinked");
    }

    /// An export carries the blocks the device has, and reads back as the
    /// same document.
    #[test]
    fn an_export_carries_the_new_blocks_and_round_trips() {
        let (mut s, _) = rig(ALL);
        let d = capture(&mut s, Some("Test".into())).unwrap();
        assert!(d.subharm.is_some() && d.tube.is_some());
        for c in &d.channels {
            assert_eq!(c.limiter.is_some(), c.is_output, "channel {}", c.channel_id);
        }
        assert_eq!(parse(&write(&d)).unwrap(), d);

        let (mut s, _) = rig(&[]);
        let d = capture(&mut s, None).unwrap();
        assert!(d.subharm.is_none() && d.tube.is_none());
        assert!(d.channels.iter().all(|c| c.limiter.is_none()));
    }
}

/// Interchange with the Consoles: the channel numbering on both platforms,
/// the fields the Console carries, and files from older Terminal builds
/// (PresetDocument.swift:615-684, PresetDocumentTransfer.swift:20-667).
#[cfg(test)]
mod interop_tests {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo, Feature};
    use dspi_proto::Platform;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, LogHandle};

    /// A device of either shape: RP2350 has eight inputs and nine outputs
    /// (config.h:773-774), RP2040 two and five (config.h:776-777).
    fn caps(platform: Platform, features: &[&str]) -> Capabilities {
        let (inputs, outputs) = match platform {
            Platform::Rp2040 => (2u8, 5u8),
            _ => (8, 9),
        };
        let n = inputs + outputs;
        Capabilities {
            serial: "TEST".into(),
            platform,
            firmware: "1.1.6 beta 4".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 6, 4),
            build_info: None,
            wire_format: dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8,
            num_channels: n,
            num_inputs: inputs,
            num_outputs: outputs,
            max_bands: 10,
            band_storage: 12,
            channels: (0..n)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= inputs,
                })
                .collect(),
            features: features
                .iter()
                .map(|n| Feature {
                    name: (*n).into(),
                    present: true,
                    evidence: "test".into(),
                })
                .collect(),
            cs: None,
            siggen: None,
            active_preset: None,
        }
    }

    const FEATURES: &[&str] = &[
        "loudness_output_mask",
        "crossfeed_output_mask",
        "leveller_masks",
        "psychoacoustic_bass",
        "upmixer",
        "lg_sound_sync",
        "subharmonic_synth",
        "tube_preamp",
        "output_limiter",
    ];

    fn rig_with(t: MockTransport, platform: Platform, features: &[&str]) -> (Session, LogHandle) {
        let log = t.log_handle();
        (
            Session::new(Box::new(t), caps(platform, features)).unwrap(),
            log,
        )
    }

    fn rig(platform: Platform) -> (Session, LogHandle) {
        rig_with(
            // The canned 64 bytes are no bulk image, and a short one fails a capture.
            MockTransport::new()
                .reply(
                    op::REQ_GET_ALL_PARAMS_CHUNK,
                    dspi_transport::mock::Reply::Stall,
                )
                .answering_everything(vec![0; 64]),
            platform,
            FEATURES,
        )
    }

    /// `(wValue, payload)` of every OUT transfer with this opcode, in order.
    fn sent(log: &LogHandle, opcode: u8) -> Vec<(u16, Vec<u8>)> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && e.opcode == opcode)
            .map(|e| (e.value, e.payload.clone()))
            .collect()
    }

    /// The unified channels that received an EQ band with this frequency.
    fn eq_channels_at(log: &LogHandle, freq: f32) -> Vec<u8> {
        let mut chs: Vec<u8> = sent(log, op::REQ_SET_EQ_PARAM)
            .iter()
            .filter(|(_, p)| p[4..8] == freq.to_le_bytes())
            .map(|(_, p)| p[0])
            .collect();
        chs.dedup();
        chs
    }

    // ------------------------------------------------------------ the table

    /// PresetDocumentTests.swift:195-240, both directions and the fixed
    /// points.
    #[test]
    fn the_channel_ids_are_the_windows_consoles() {
        for i in 0..8 {
            assert_eq!(
                channel_id::resolve(channel_id::for_input(i)),
                Some(ChannelRef::Input(i))
            );
        }
        for o in 0..9 {
            assert_eq!(
                channel_id::resolve(channel_id::for_output(o)),
                Some(ChannelRef::Output(o))
            );
        }
        assert_eq!(channel_id::for_input(0), 0);
        assert_eq!(channel_id::for_input(1), 1);
        assert_eq!(channel_id::for_input(2), 11);
        assert_eq!(channel_id::for_input(7), 16);
        assert_eq!(channel_id::for_output(0), 2);
        assert_eq!(channel_id::for_output(4), 6, "RP2040's PDM");
        assert_eq!(channel_id::for_output(8), 10, "RP2350's PDM");
        assert_eq!(channel_id::resolve(-1), None);
    }

    /// The Console's own index fields win over the shared id
    /// (PresetDocumentTests.swift:244-253).
    #[test]
    fn the_consoles_index_fields_win_over_the_shared_id() {
        let mut b = ChannelBlock {
            channel_id: 2,
            is_output: true,
            output_index: Some(5),
            ..Default::default()
        };
        assert_eq!(b.placement(None), Some(ChannelRef::Output(5)));
        b.output_index = None;
        assert_eq!(b.placement(None), Some(ChannelRef::Output(0)));
        // An input's own index wins too, and an output index on an input
        // entry is not read.
        let b = ChannelBlock {
            channel_id: 11,
            input_index: Some(3),
            output_index: Some(1),
            ..Default::default()
        };
        assert_eq!(b.placement(None), Some(ChannelRef::Input(3)));
    }

    // ------------------------------------------------- Console-written files

    /// An RP2350 document as the Console writes one: the Windows id, the
    /// unified `eqChannel`, and its own index (PresetDocumentTransfer.swift:
    /// 166-197). The ids and the unified numbers differ for every channel but
    /// the first two, so a placement by either would show here.
    const CONSOLE_RP2350: &str = r#"{
      "schemaVersion": 1,
      "meta": {"platform": "RP2350", "savedUtc": "2026-09-29T10:00:00Z",
               "inputChannelCount": 8, "outputChannelCount": 9, "wireFormatVersion": 32},
      "channels": [
        {"channelId": 0, "eqChannel": 0, "inputIndex": 0, "isOutput": false, "name": "USB 1",
         "delayMs": 0, "eq": [{"type": 1, "freqHz": 100, "q": 1, "gain": 3}], "crossover": []},
        {"channelId": 11, "eqChannel": 2, "inputIndex": 2, "isOutput": false, "name": "USB 3",
         "delayMs": 1, "eq": [{"type": 1, "freqHz": 102, "q": 1, "gain": 3}], "crossover": []},
        {"channelId": 16, "eqChannel": 7, "inputIndex": 7, "isOutput": false, "name": "USB 8",
         "delayMs": 0, "eq": [{"type": 1, "freqHz": 107, "q": 1, "gain": 3}], "crossover": []},
        {"channelId": 2, "eqChannel": 8, "outputIndex": 0, "isOutput": true, "name": "SPDIF 1 L",
         "gainDb": -2, "muted": false, "enabled": true, "outputDelayMs": 1.5, "delayMs": 0,
         "eq": [{"type": 1, "freqHz": 200, "q": 1, "gain": 3}],
         "crossover": [{"type": 34, "freqHz": 80, "q": 0.707, "gain": 0}]},
        {"channelId": 10, "eqChannel": 16, "outputIndex": 8, "isOutput": true, "name": "PDM",
         "gainDb": -4, "muted": false, "enabled": true, "outputDelayMs": 0, "delayMs": 0,
         "eq": [{"type": 1, "freqHz": 208, "q": 1, "gain": 3}], "crossover": []}
      ]
    }"#;

    /// The same channels as a Windows file carries them: the shared id and
    /// nothing else (PresetDocumentTests.swift:260-323).
    const WINDOWS_RP2350: &str = r#"{
      "schemaVersion": 1,
      "meta": {"platform": "RP2350", "savedUtc": "2026-07-14T09:31:07.4821563+00:00",
               "inputChannelCount": 8, "outputChannelCount": 9},
      "channels": [
        {"channelId": 0, "name": "Master L", "isOutput": false, "eq": [{"type": 1, "freqHz": 100}]},
        {"channelId": 11, "name": "Input 3", "isOutput": false, "eq": [{"type": 1, "freqHz": 102}]},
        {"channelId": 16, "name": "Input 8", "isOutput": false, "eq": [{"type": 1, "freqHz": 107}]},
        {"channelId": 2, "name": "SPDIF 1 L", "isOutput": true, "gainDb": -2,
         "eq": [{"type": 1, "freqHz": 200}]},
        {"channelId": 10, "name": "PDM", "isOutput": true, "gainDb": -4,
         "eq": [{"type": 1, "freqHz": 208}]}
      ]
    }"#;

    /// An RP2040 document as the Console writes one. Here the id and the
    /// unified index coincide, and PDM is the fifth output at id 6.
    const CONSOLE_RP2040: &str = r#"{
      "schemaVersion": 1,
      "meta": {"platform": "RP2040", "savedUtc": "2026-09-29T10:00:00Z",
               "inputChannelCount": 2, "outputChannelCount": 5},
      "channels": [
        {"channelId": 0, "eqChannel": 0, "inputIndex": 0, "isOutput": false, "name": "USB L",
         "eq": [{"type": 1, "freqHz": 100}]},
        {"channelId": 1, "eqChannel": 1, "inputIndex": 1, "isOutput": false, "name": "USB R",
         "eq": [{"type": 1, "freqHz": 101}]},
        {"channelId": 2, "eqChannel": 2, "outputIndex": 0, "isOutput": true, "name": "SPDIF 1 L",
         "gainDb": -2, "eq": [{"type": 1, "freqHz": 200}]},
        {"channelId": 6, "eqChannel": 6, "outputIndex": 4, "isOutput": true, "name": "PDM",
         "gainDb": -4, "eq": [{"type": 1, "freqHz": 204}]}
      ]
    }"#;

    fn places(text: &str, inputs: u8, outputs: u8) -> Vec<(ChannelRef, String)> {
        let doc = parse(text).unwrap();
        let (placed, missing) = place_channels(&doc, inputs, outputs);
        assert!(missing.is_empty(), "{missing:?}");
        placed.iter().map(|(r, c)| (*r, c.name.clone())).collect()
    }

    #[test]
    fn a_console_document_places_every_channel_on_rp2350() {
        use ChannelRef::*;
        for text in [CONSOLE_RP2350, WINDOWS_RP2350] {
            let got: Vec<ChannelRef> = places(text, 8, 9).into_iter().map(|p| p.0).collect();
            assert_eq!(
                got,
                vec![Input(0), Input(2), Input(7), Output(0), Output(8)],
                "{text}"
            );
        }
    }

    #[test]
    fn a_console_document_places_every_channel_on_rp2040() {
        use ChannelRef::*;
        let got: Vec<ChannelRef> = places(CONSOLE_RP2040, 2, 5)
            .into_iter()
            .map(|p| p.0)
            .collect();
        assert_eq!(got, vec![Input(0), Input(1), Output(0), Output(4)]);
    }

    /// Applied to an RP2350, each channel's EQ reaches its unified channel
    /// (config.h:788-790) and each output strip its output index.
    #[test]
    fn a_console_document_applies_to_the_right_channels_on_rp2350() {
        for text in [CONSOLE_RP2350, WINDOWS_RP2350] {
            let (mut s, log) = rig(Platform::Rp2350);
            let report = apply(&mut s, &parse(text).unwrap(), ApplyOptions::default());
            assert!(report.missing_channels.is_empty());
            assert_eq!(report.channels_applied, 5);
            for (freq, unified) in [(100.0, 0), (102.0, 2), (107.0, 7), (200.0, 8), (208.0, 16)] {
                assert_eq!(eq_channels_at(&log, freq), vec![unified], "{freq} Hz");
            }
            let gains: Vec<(u16, Vec<u8>)> = sent(&log, op::REQ_SET_OUTPUT_GAIN);
            assert_eq!(
                gains,
                vec![
                    (0, (-2.0f32).to_le_bytes().to_vec()),
                    (8, (-4.0f32).to_le_bytes().to_vec())
                ]
            );
        }
    }

    #[test]
    fn a_console_document_applies_to_the_right_channels_on_rp2040() {
        let (mut s, log) = rig(Platform::Rp2040);
        let report = apply(
            &mut s,
            &parse(CONSOLE_RP2040).unwrap(),
            ApplyOptions::default(),
        );
        assert!(report.missing_channels.is_empty());
        for (freq, unified) in [(100.0, 0), (101.0, 1), (200.0, 2), (204.0, 6)] {
            assert_eq!(eq_channels_at(&log, freq), vec![unified], "{freq} Hz");
        }
        let gains: Vec<u16> = sent(&log, op::REQ_SET_OUTPUT_GAIN)
            .iter()
            .map(|g| g.0)
            .collect();
        assert_eq!(gains, vec![0, 4]);
    }

    /// An eight-input document on a two-input part reports what the part
    /// lacks rather than folding it onto what it has.
    #[test]
    fn an_rp2350_document_on_an_rp2040_reports_what_is_missing() {
        let doc = parse(CONSOLE_RP2350).unwrap();
        let (placed, missing) = place_channels(&doc, 2, 5);
        assert_eq!(
            placed.keys().copied().collect::<Vec<_>>(),
            vec![ChannelRef::Input(0), ChannelRef::Output(0)]
        );
        assert_eq!(missing, vec!["USB 3", "USB 8", "PDM"]);
    }

    /// The post-matrix output delay and the channel name travel too.
    #[test]
    fn the_output_delay_and_names_are_applied() {
        let (mut s, log) = rig(Platform::Rp2350);
        apply(
            &mut s,
            &parse(CONSOLE_RP2350).unwrap(),
            ApplyOptions::default(),
        );
        let delays = sent(&log, op::REQ_SET_OUTPUT_DELAY);
        assert_eq!(delays[0], (0, 1.5f32.to_le_bytes().to_vec()));
        let names: Vec<(u16, String)> = sent(&log, op::REQ_SET_CHANNEL_NAME)
            .into_iter()
            .map(|(v, p)| (v, String::from_utf8_lossy(&p).trim_end_matches('\0').into()))
            .collect();
        assert!(names.contains(&(2, "USB 3".into())), "{names:?}");
        assert!(names.contains(&(16, "PDM".into())), "{names:?}");
    }

    /// Bands a document carries replace the channel's; the rest of the bank
    /// goes flat, and an empty list leaves the channel alone
    /// (PresetDocumentTransfer.swift:474-502).
    #[test]
    fn a_partial_bank_flattens_the_rest_and_an_empty_one_is_left_alone() {
        let (mut s, log) = rig(Platform::Rp2350);
        apply(
            &mut s,
            &parse(CONSOLE_RP2350).unwrap(),
            ApplyOptions::default(),
        );
        let eq = sent(&log, op::REQ_SET_EQ_PARAM);
        let on = |ch: u8| eq.iter().filter(|(_, p)| p[0] == ch).count();
        // Ten EQ bands on input 0; ten plus four crossover on output 0.
        assert_eq!(on(0), 10);
        assert_eq!(on(8), 14);
        // Output 8 carries no crossover, so only its EQ is written.
        assert_eq!(on(16), 10);
        // Input 1 is not in the document at all.
        assert_eq!(on(1), 0);
    }

    // ---------------------------------------------- the fields it now carries

    /// The masks and the upmix modes, each where the firmware has one.
    #[test]
    fn masks_and_upmix_modes_are_applied() {
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        doc.loudness.output_mask = Some(0x0003);
        doc.crossfeed.output_pair_mask = Some(0x02);
        doc.leveller.detector_mask = Some(0x03);
        doc.leveller.apply_mask = Some(0x02);
        doc.psybass = Some(PsybassBlock {
            output_mask: Some(0x0100),
            ..Default::default()
        });
        doc.upmix = Some(UpmixBlock {
            enabled: Some(false),
            center_mode: Some(0),
            surround_mode: Some(1),
            strength_pct: Some(100.0),
            center_width_pct: Some(50.0),
            threshold_pct: Some(40.0),
            attack_ms: Some(5.0),
            release_ms: Some(100.0),
            detector_hpf_hz: Some(100.0),
            surround_delay_ms: Some(10.0),
            surround_hpf_hz: Some(100.0),
            surround_lpf_hz: Some(6000.0),
            decorr_pct: Some(50.0),
            presence_db: Some(0.0),
        });
        doc.global.lg_sound_sync_enabled = Some(true);

        let (mut s, log) = rig(Platform::Rp2350);
        let report = apply(&mut s, &doc, ApplyOptions::default());
        assert!(
            !report.skipped.iter().any(|r| r.contains("not supported")),
            "{:?}",
            report.skipped
        );

        assert_eq!(sent(&log, op::REQ_SET_LOUDNESS_MASK)[0].1, vec![0x03, 0x00]);
        assert_eq!(sent(&log, op::REQ_SET_CROSSFEED_OUTPUTS)[0].1, vec![0x02]);
        assert_eq!(
            sent(&log, op::REQ_SET_LEVELLER_MASKS)[0].1,
            vec![0x03, 0x02]
        );
        assert_eq!(sent(&log, op::REQ_SET_PSYBASS_MASK)[0].1, vec![0x00, 0x01]);
        assert_eq!(sent(&log, op::REQ_SET_LG_SOUND_SYNC_ENABLE)[0].1, vec![1]);

        // UPMIX_PARAM_* by wValue (upmix.h:188-201), every one a float.
        let up: Vec<(u16, Vec<u8>)> = sent(&log, op::REQ_UPMIX_SET_PARAM);
        let wvalues: Vec<u16> = up.iter().map(|u| u.0).collect();
        assert_eq!(wvalues, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0]);
        assert_eq!(up[0].1, 0.0f32.to_le_bytes().to_vec());
        assert_eq!(up[1].1, 1.0f32.to_le_bytes().to_vec());
        assert_eq!(up[4].1, 40.0f32.to_le_bytes().to_vec());
        assert_eq!(up[10].1, 6000.0f32.to_le_bytes().to_vec());
    }

    /// Each feature's parameters go before its switch
    /// (PresetDocumentTransfer.swift:557-584).
    #[test]
    fn every_switch_follows_its_parameters() {
        let (mut s, log) = rig(Platform::Rp2350);
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        doc.loudness.output_mask = Some(0x03);
        doc.loudness.enabled = Some(true);
        doc.crossfeed.output_pair_mask = Some(0x01);
        doc.crossfeed.enabled = Some(true);
        doc.leveller.detector_mask = Some(0xFF);
        doc.leveller.apply_mask = Some(0xFF);
        doc.leveller.enabled = Some(true);
        apply(&mut s, &doc, ApplyOptions::default());
        let order: Vec<u8> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out)
            .map(|e| e.opcode)
            .collect();
        let at = |o: u8| order.iter().position(|x| *x == o).unwrap();
        assert!(at(op::REQ_SET_LOUDNESS_MASK) < at(op::REQ_SET_LOUDNESS));
        assert!(at(op::REQ_SET_CROSSFEED_OUTPUTS) < at(op::REQ_SET_CROSSFEED));
        assert!(at(op::REQ_SET_LEVELLER_MASKS) < at(op::REQ_SET_LEVELLER_ENABLE));
    }

    /// A feature the device lacks is skipped in the Console's words, and a
    /// mask the firmware predates is not sent.
    #[test]
    fn missing_features_are_skipped_in_the_consoles_words() {
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        doc.psybass = Some(PsybassBlock::default());
        doc.upmix = Some(UpmixBlock::default());
        doc.global.lg_sound_sync_enabled = Some(true);
        let (mut s, log) = rig_with(
            MockTransport::new().answering_everything(vec![0; 64]),
            Platform::Rp2350,
            &[],
        );
        let report = apply(&mut s, &doc, ApplyOptions::default());
        for reason in [
            "Psychoacoustic bass (not supported by this firmware)",
            "Stereo upmixer (not supported by this device)",
            "LG Sound Sync (not supported by this firmware)",
        ] {
            assert!(report.skipped.iter().any(|r| r == reason), "{reason}");
        }
        for opcode in [
            op::REQ_SET_LOUDNESS_MASK,
            op::REQ_SET_CROSSFEED_OUTPUTS,
            op::REQ_SET_LEVELLER_MASKS,
            op::REQ_UPMIX_SET_PARAM,
        ] {
            assert!(sent(&log, opcode).is_empty(), "0x{opcode:02X}");
        }
    }

    /// A Windows file from before the masks existed leaves the device's alone,
    /// rather than zero, which would switch loudness off everywhere, or a default
    /// the device may not hold (WP TM21).
    #[test]
    fn absent_masks_leave_the_devices_alone() {
        let d = parse(
            r#"{"schemaVersion":1,"loudness":{"enabled":true},"crossfeed":{"enabled":true},
                "leveller":{},"upmix":{},"psybass":{},"channels":[{"channelId":0}]}"#,
        )
        .unwrap();
        assert_eq!(d.loudness.output_mask, None);
        assert_eq!(d.crossfeed.output_pair_mask, None);
        let (mut s, log) = rig(Platform::Rp2350);
        let report = apply(&mut s, &d, ApplyOptions::default());
        for opcode in [
            op::REQ_SET_LOUDNESS_MASK,
            op::REQ_SET_LOUDNESS_REF,
            op::REQ_SET_CROSSFEED_OUTPUTS,
            op::REQ_SET_LEVELLER_MASKS,
            op::REQ_SET_PSYBASS_MASK,
            op::REQ_UPMIX_SET_PARAM,
        ] {
            assert!(sent(&log, opcode).is_empty(), "0x{opcode:02X}");
        }
        assert_eq!(
            sent(&log, op::REQ_SET_LOUDNESS).len(),
            1,
            "the file said this"
        );
        assert!(
            report
                .left_alone
                .contains(&"loudness.outputMask".to_string())
        );
    }

    /// Master volume is a preset's only in with-preset mode
    /// (PresetDocumentTransfer.swift:669-680); listening volume always is.
    #[test]
    fn master_volume_follows_the_devices_mode() {
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        doc.global.master_volume_db = Some(-6.0);
        doc.global.user_volume_db = Some(-3.0);
        let options = ApplyOptions {
            volume_levels: true,
            ..Default::default()
        };

        let (mut s, log) = rig(Platform::Rp2350);
        let report = apply(&mut s, &doc, options);
        assert!(sent(&log, op::REQ_SET_MASTER_VOLUME).is_empty());
        assert_eq!(sent(&log, op::REQ_SET_USER_VOLUME).len(), 1);
        assert!(
            report
                .skipped
                .contains(&"Master volume (device is in independent master-volume mode)".into())
        );

        let (mut s, log) = rig_with(
            MockTransport::new()
                .answering_everything(vec![0; 64])
                .data(op::REQ_GET_MASTER_VOLUME_MODE, vec![1]),
            Platform::Rp2350,
            FEATURES,
        );
        apply(&mut s, &doc, options);
        assert_eq!(sent(&log, op::REQ_SET_MASTER_VOLUME).len(), 1);
    }

    /// Disables first, then enables, so an output the document switches off
    /// is free before the one it switches on needs it
    /// (PresetDocumentTransfer.swift:377-392).
    #[test]
    fn outputs_are_disabled_before_any_is_enabled() {
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        // The document turns output 0 off and PDM on; the device holds
        // output 0 on and PDM off.
        doc.channels[3].enabled = Some(false);
        doc.channels[4].enabled = Some(true);
        use dspi_transport::mock::Reply;
        let reads = |v: &[u8]| v.iter().map(|b| Reply::Data(vec![*b])).collect::<Vec<_>>();
        let (mut s, log) = rig_with(
            MockTransport::new()
                .answering_everything(vec![0; 64])
                // Output 0: the check, the write's own read, its readback;
                // then the same three for PDM.
                .reply(
                    op::REQ_GET_OUTPUT_ENABLE,
                    Reply::Sequence(reads(&[1, 1, 0, 0, 0, 1])),
                ),
            Platform::Rp2350,
            FEATURES,
        );
        apply(&mut s, &doc, ApplyOptions::default());
        let enables: Vec<(u16, u8)> = sent(&log, op::REQ_SET_OUTPUT_ENABLE)
            .iter()
            .map(|(v, p)| (*v, p[0]))
            .collect();
        assert_eq!(enables, vec![(0, 0), (8, 1)]);
    }

    /// An enable that still collides with Core 1 is reported, not forced.
    #[test]
    fn a_colliding_enable_is_reported() {
        let mut doc = parse(CONSOLE_RP2350).unwrap();
        doc.channels[4].enabled = Some(true);
        let (mut s, log) = rig_with(
            MockTransport::new()
                .answering_everything(vec![0; 64])
                .data(op::REQ_GET_CORE1_CONFLICT, vec![1]),
            Platform::Rp2350,
            FEATURES,
        );
        let report = apply(&mut s, &doc, ApplyOptions::default());
        assert!(
            report
                .skipped
                .contains(&"PDM could not be enabled (conflicts with another output)".into()),
            "{:?}",
            report.skipped
        );
        assert!(
            !sent(&log, op::REQ_SET_OUTPUT_ENABLE)
                .iter()
                .any(|(_, p)| p[0] == 1)
        );
    }

    // ------------------------------------------------------------- exports

    /// An export numbers channels as the Console does and carries its keys
    /// (PresetDocumentTests.swift:337-433, PresetDocumentTransfer.swift:
    /// 166-197).
    #[test]
    fn an_rp2350_export_uses_the_consoles_numbering_and_keys() {
        let (mut s, _) = rig(Platform::Rp2350);
        let d = capture(&mut s, Some("Test".into())).unwrap();

        let ids: Vec<i32> = d.channels.iter().map(|c| c.channel_id).collect();
        assert_eq!(
            ids,
            vec![0, 1, 11, 12, 13, 14, 15, 16, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        );
        for (unified, c) in d.channels.iter().enumerate() {
            assert_eq!(c.eq_channel, Some(unified as i32));
            if c.is_output {
                assert_eq!(c.output_index, Some(unified as i32 - 8));
                assert_eq!(c.input_index, None);
                assert!(c.output_delay_ms.is_some());
            } else {
                assert_eq!(c.input_index, Some(unified as i32));
                assert_eq!(c.output_index, None);
            }
        }

        let json: serde_json::Value = serde_json::from_str(&write(&d)).unwrap();
        let keys = |v: &serde_json::Value| -> Vec<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        let has_all = |v: &serde_json::Value, want: &[&str]| {
            let got = keys(v);
            for k in want {
                assert!(got.iter().any(|g| g == k), "missing {k} in {got:?}");
            }
        };
        has_all(
            &json,
            &[
                "schemaVersion",
                "meta",
                "global",
                "loudness",
                "crossfeed",
                "leveller",
                "psybass",
                "upmix",
                "subharm",
                "tube",
                "channels",
                "matrix",
                "io",
            ],
        );
        has_all(
            &json["meta"],
            &[
                "name",
                "savedUtc",
                "appVersion",
                "platform",
                "firmwareVersion",
                "wireFormatVersion",
                "inputChannelCount",
                "outputChannelCount",
                "masterVolumeMode",
                "outputConfigMode",
            ],
        );
        has_all(
            &json["upmix"],
            &[
                "enabled",
                "centerMode",
                "surroundMode",
                "strengthPct",
                "centerWidthPct",
                "thresholdPct",
                "attackMs",
                "releaseMs",
                "detectorHpfHz",
                "surroundDelayMs",
                "surroundHpfHz",
                "surroundLpfHz",
                "decorrPct",
                "presenceDb",
            ],
        );
        has_all(
            &json["channels"][2],
            &["channelId", "eqChannel", "inputIndex", "name"],
        );
        has_all(
            &json["channels"][16],
            &[
                "channelId",
                "name",
                "isOutput",
                "delayMs",
                "gainDb",
                "muted",
                "enabled",
                "eq",
                "crossover",
                "eqChannel",
                "outputIndex",
                "outputDelayMs",
                "limiter",
            ],
        );
        assert_eq!(json["channels"][16]["channelId"], 10, "PDM");
        assert_eq!(json["channels"][16]["outputIndex"], 8);
        assert_eq!(json["channels"][16]["eqChannel"], 16);
        let saved = json["meta"]["savedUtc"].as_str().unwrap();
        assert_eq!(saved.len(), 20, "{saved}");
        assert!(saved.ends_with('Z'));
    }

    #[test]
    fn an_rp2040_export_uses_the_consoles_numbering() {
        let (mut s, _) = rig(Platform::Rp2040);
        let d = capture(&mut s, None).unwrap();
        let ids: Vec<(i32, Option<i32>, Option<i32>)> = d
            .channels
            .iter()
            .map(|c| (c.channel_id, c.input_index, c.output_index))
            .collect();
        assert_eq!(
            ids,
            vec![
                (0, Some(0), None),
                (1, Some(1), None),
                (2, None, Some(0)),
                (3, None, Some(1)),
                (4, None, Some(2)),
                (5, None, Some(3)),
                (6, None, Some(4)),
            ]
        );
    }

    /// An export read back lands every channel where it came from, on both
    /// platforms, whether read by its own index fields or, as a Windows
    /// reader does, by `channelId` alone.
    #[test]
    fn an_export_round_trips_on_both_platforms() {
        for (platform, inputs, outputs) in [(Platform::Rp2350, 8, 9), (Platform::Rp2040, 2, 5)] {
            let (mut s, _) = rig(platform);
            let d = parse(&write(&capture(&mut s, None).unwrap())).unwrap();
            let (placed, missing) = place_channels(&d, inputs, outputs);
            assert!(missing.is_empty());
            assert_eq!(placed.len(), (inputs + outputs) as usize);
            for (r, c) in &placed {
                assert_eq!(Some(r.unified(inputs) as i32), c.eq_channel, "{platform:?}");
                // What a Windows reader sees.
                assert_eq!(channel_id::resolve(c.channel_id), Some(*r));
            }

            // Applied back, every channel's bank reaches its own channel.
            let (mut s, log) = rig(platform);
            apply(&mut s, &d, ApplyOptions::default());
            let mut chs: Vec<u8> = sent(&log, op::REQ_SET_EQ_PARAM)
                .iter()
                .map(|(_, p)| p[0])
                .collect();
            chs.dedup();
            assert_eq!(chs, (0..inputs + outputs).collect::<Vec<_>>());
        }
    }

    // --------------------------------------------- older Terminal documents

    /// An eight-input file an older Terminal build wrote: the unified index
    /// as `channelId`, no index fields, no timestamp.
    fn older_terminal_rp2350() -> String {
        let chans: Vec<String> = (0..17)
            .map(|i| {
                format!(
                    r#"{{"channelId":{i},"name":"Ch {i}","isOutput":{},"eq":[{{"type":1,"freqHz":{}}}]}}"#,
                    i >= 8,
                    300 + i
                )
            })
            .collect();
        format!(
            r#"{{"schemaVersion":1,"meta":{{"appVersion":"2.0.0-dev","platform":"RP2350",
                "inputChannelCount":8,"outputChannelCount":9}},"channels":[{}]}}"#,
            chans.join(",")
        )
    }

    #[test]
    fn an_older_terminal_document_is_recognised_and_placed() {
        let d = parse(&older_terminal_rp2350()).unwrap();
        assert_eq!(legacy_numbering(&d, 8), Some(8));
        let (placed, missing) = place_channels(&d, 8, 9);
        assert!(missing.is_empty());
        for (r, c) in &placed {
            assert_eq!(r.unified(8) as i32, c.channel_id, "{}", c.name);
        }

        let (mut s, log) = rig(Platform::Rp2350);
        apply(&mut s, &d, ApplyOptions::default());
        for i in 0..17u8 {
            assert_eq!(eq_channels_at(&log, 300.0 + i as f32), vec![i]);
        }
    }

    /// No file either Console writes looks like an older Terminal one, and on
    /// a two-input device the two numberings are the same thing.
    #[test]
    fn console_documents_are_never_taken_for_older_terminal_ones() {
        for text in [CONSOLE_RP2350, WINDOWS_RP2350, CONSOLE_RP2040] {
            assert_eq!(legacy_numbering(&parse(text).unwrap(), 8), None, "{text}");
        }
        let (mut s, _) = rig(Platform::Rp2350);
        let exported = capture(&mut s, None).unwrap();
        assert_eq!(legacy_numbering(&exported, 8), None);
        // An older RP2040 file already agrees with the table.
        let old = r#"{"schemaVersion":1,"meta":{"inputChannelCount":2},"channels":[
            {"channelId":0},{"channelId":1},{"channelId":2,"isOutput":true},
            {"channelId":6,"isOutput":true}]}"#;
        assert_eq!(legacy_numbering(&parse(old).unwrap(), 2), None);
        assert_eq!(
            place_channels(&parse(old).unwrap(), 2, 5)
                .0
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![
                ChannelRef::Input(0),
                ChannelRef::Input(1),
                ChannelRef::Output(0),
                ChannelRef::Output(4)
            ]
        );
    }

    #[test]
    fn the_timestamp_is_iso_8601_utc() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(1_790_762_645), "2026-09-30T10:04:05Z");
    }
}

#[cfg(test)]
mod wp_tm10_tests {
    use super::*;

    const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");
    const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

    /// Every key and value of `a` is in `b`. Numbers compare as `f32`, since
    /// that is what the writers hold.
    fn assert_kept(a: &serde_json::Value, b: &serde_json::Value, path: &str) {
        use serde_json::Value as V;
        match (a, b) {
            (V::Object(x), V::Object(y)) => {
                for (k, v) in x {
                    if v.is_null() {
                        continue;
                    }
                    let other = y.get(k);
                    assert!(other.is_some(), "{path}.{k} was dropped");
                    assert_kept(v, other.unwrap(), &format!("{path}.{k}"));
                }
            }
            (V::Array(x), V::Array(y)) => {
                assert_eq!(x.len(), y.len(), "{path} length");
                for (i, (v, w)) in x.iter().zip(y).enumerate() {
                    assert_kept(v, w, &format!("{path}[{i}]"));
                }
            }
            (V::Number(x), V::Number(y)) => {
                assert_eq!(
                    x.as_f64().unwrap() as f32,
                    y.as_f64().unwrap() as f32,
                    "{path}"
                );
            }
            _ => assert_eq!(a, b, "{path}"),
        }
    }

    #[test]
    fn wp_tm10_the_shared_documents_keep_every_key_through_this_parser() {
        for (name, text) in [("qt", QT), ("mac", MAC)] {
            let doc = parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let again = write(&doc);
            assert_eq!(parse(&again).unwrap(), doc, "{name}");
            let a: serde_json::Value = serde_json::from_str(text).unwrap();
            let b: serde_json::Value = serde_json::from_str(&again).unwrap();
            assert_kept(&a, &b, name);
        }
    }

    #[test]
    fn wp_tm10_the_pair_map_is_read_as_the_console_reads_it() {
        let block = |raw: &str| -> CrossfeedBlock {
            let doc = format!(
                r#"{{"schemaVersion":1,"crossfeed":{{"pairMap":{raw}}},"channels":[{{"channelId":0}}]}}"#
            );
            parse(&doc).unwrap().crossfeed
        };
        // K340's map, with explicit default bytes normalized away.
        assert_eq!(
            block("[2, 19, 84, 118]").pair_map_for(4),
            Some(PairMap([0x02, 0x13, 0, 0]))
        );
        // Too wide for the part or duplicated outputs: the default, not an error.
        assert_eq!(
            block("[2, 19, 0, 0]").pair_map_for(1),
            Some(PairMap([0; 4]))
        );
        assert_eq!(block("[2, 2, 0, 0]").pair_map_for(4), Some(PairMap([0; 4])));
        assert_eq!(
            block("[0, 0, 0, 0, 1]").pair_map_for(4),
            Some(PairMap([0; 4]))
        );
        // Not a list reads as absent, and that leaves the device alone.
        assert_eq!(block("\"nope\"").pair_map, None);
        assert_eq!(block("null").pair_map_for(4), None);
        assert_eq!(CrossfeedBlock::default().pair_map_for(4), None);
    }

    #[test]
    fn wp_tm10_sniff_tells_the_documents_apart() {
        assert_eq!(sniff(QT), DocumentKind::Preset);
        assert_eq!(sniff(MAC), DocumentKind::Preset);
        assert_eq!(
            sniff("\u{feff} {\"schemaVersion\": 1}"),
            DocumentKind::Preset
        );
        assert_eq!(sniff("{broken"), DocumentKind::Preset);
        assert_eq!(
            sniff(r#"{"kind": "dspi-filters", "channels": []}"#),
            DocumentKind::FilterJson
        );
        assert_eq!(
            sniff(r#"{"kind": "dspi-control-surfaces"}"#),
            DocumentKind::ControlSurfaces
        );
        assert_eq!(
            sniff("Filter 1: ON PK Fc 100 Hz Gain 3 dB Q 1"),
            DocumentKind::FilterText
        );
        assert_eq!(sniff(""), DocumentKind::FilterText);
    }

    #[test]
    fn wp_tm10_a_filter_document_round_trips_with_the_text_format() {
        for text in [
            include_str!("../../../tests/fixtures/DSPiFilters.txt"),
            include_str!("../../../tests/fixtures/MacOSFilters.txt"),
            include_str!("../../../tests/fixtures/WindowsFilters.txt"),
            include_str!("../../../tests/fixtures/REWFilters.txt"),
        ] {
            let file = crate::filterfile::parse(text).unwrap();
            let doc = FilterDocument::from_text(&file);
            let again = parse_filters(&doc.write()).unwrap();
            assert_eq!(again, doc);
            let back = doc.to_text();
            assert_eq!(back.channels, file.channels);
            // And through the text writer, which is what `export` runs.
            let text = crate::filterfile::write(&back, "now");
            let reread = FilterDocument::from_text(&crate::filterfile::parse(&text).unwrap());
            assert_eq!(reread.channels.len(), doc.channels.len());
        }
    }

    #[test]
    fn wp_tm10_bad_filter_documents_are_refused_with_a_reason() {
        assert_eq!(parse_filters("  "), Err(PresetFileError::Empty));
        assert!(matches!(
            parse_filters(r#"{"kind":"x","formatVersion":2}"#),
            Err(PresetFileError::Invalid(_))
        ));
        assert_eq!(
            parse_filters(r#"{"kind":"dspi-filters","formatVersion":99,"channels":[{}]}"#),
            Err(PresetFileError::FromTheFuture {
                got: 99,
                supported: 2
            })
        );
        assert_eq!(
            parse_filters(r#"{"kind":"dspi-filters","formatVersion":2}"#),
            Err(PresetFileError::NoChannels)
        );
    }

    #[test]
    fn wp_tm10_bad_control_surface_documents_are_refused_with_a_reason() {
        assert!(matches!(
            parse_control_surfaces(r#"{"kind":"x","schemaVersion":1}"#),
            Err(PresetFileError::Invalid(_))
        ));
        assert_eq!(
            parse_control_surfaces(r#"{"kind":"dspi-control-surfaces","schemaVersion":2}"#),
            Err(PresetFileError::FromTheFuture {
                got: 2,
                supported: 1
            })
        );
        // A number out of its field's range does not wrap.
        assert!(matches!(
            parse_control_surfaces(
                r#"{"kind":"dspi-control-surfaces","schemaVersion":1,"bindings":[{"noun":300}]}"#
            ),
            Err(PresetFileError::Invalid(_))
        ));
        // A partial binding keeps the second pin unused, not GPIO 0.
        let doc = parse_control_surfaces(
            r#"{"kind":"dspi-control-surfaces","schemaVersion":1,"bindings":[{"type":1,"gpio0":3}]}"#,
        )
        .unwrap();
        assert_eq!(CsBinding::from(&doc.bindings[0]).gpio, [3, GPIO_UNUSED]);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod wp_tm21_tests {
    use super::*;
    use crate::probe::{Capabilities, ChannelInfo, Feature};
    use dspi_proto::Platform;
    use dspi_proto::generated::opcodes as op;
    use dspi_transport::mock::{Direction, LogHandle, Reply};
    use dspi_transport::{MockTransport, Transport};

    const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");
    const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

    fn feature(name: &str) -> Feature {
        Feature {
            name: name.into(),
            present: true,
            evidence: "test".into(),
        }
    }

    fn caps() -> Capabilities {
        Capabilities {
            serial: "TEST".into(),
            platform: Platform::Rp2350,
            firmware: "1.1.6".into(),
            firmware_version: dspi_proto::packets::FirmwareVersion::new(1, 1, 6, 0),
            build_info: None,
            wire_format: WIRE_FORMAT_VERSION as u8,
            num_channels: 5,
            num_inputs: 2,
            num_outputs: 3,
            max_bands: 2,
            band_storage: 12,
            channels: (0..5)
                .map(|i| ChannelInfo {
                    index: i,
                    name: format!("Ch {i}"),
                    slug: format!("ch.{i}"),
                    is_output: i >= 2,
                })
                .collect(),
            features: Vec::new(),
            cs: None,
            siggen: None,
            active_preset: None,
        }
    }

    /// A device that answers every read with -6.0, and what `tune` makes of it.
    fn device(tune: impl FnOnce(MockTransport) -> MockTransport) -> (Session, LogHandle) {
        let t = tune(
            MockTransport::new()
                .reply(op::REQ_GET_ALL_PARAMS_CHUNK, Reply::Stall)
                .answering_everything((-6.0f32).to_le_bytes().repeat(4)),
        );
        let log = t.log_handle();
        (Session::new(Box::new(t), caps()).unwrap(), log)
    }

    fn sets(log: &LogHandle, opcode: u8) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.direction == Direction::Out && e.opcode == opcode)
            .count()
    }

    #[test]
    fn wp_tm21_every_kind_of_failed_read_fails_the_capture() {
        for (name, reply) in [
            ("lost device", Reply::Disconnect),
            (
                "timeout",
                Reply::Error(TransportError::TimedOut {
                    opcode: op::REQ_GET_OUTPUT_GAIN,
                    retries: 4,
                }),
            ),
            ("usb", Reply::Error(TransportError::Usb("io".into()))),
            (
                "short",
                Reply::Error(TransportError::ShortRead {
                    opcode: op::REQ_GET_OUTPUT_GAIN,
                    wanted: 4,
                    got: 1,
                }),
            ),
        ] {
            let (mut s, _) = device(|m| m.reply(op::REQ_GET_OUTPUT_GAIN, reply));
            let e = capture(&mut s, None).expect_err(name);
            assert!(e.what.contains("out.gain"), "{name}: {e}");
            assert!(e.to_string().contains("could not read"), "{name}: {e}");
        }
    }

    /// A read partway through, of each shape the capture has, not just `read`.
    #[test]
    fn wp_tm21_a_failed_read_anywhere_fails_the_capture() {
        for opcode in [
            op::REQ_GET_EQ_PARAM,
            op::REQ_GET_ALL_PARAMS_CHUNK,
            op::REQ_GET_SPDIF_INPUT_CONFIG,
            op::REQ_GET_OUTPUT_TYPE,
            op::REQ_EXTENSION,
            op::REQ_GET_TUBE_PARAM,
        ] {
            let (mut s, _) = device(|m| m.reply(opcode, Reply::Disconnect));
            // The tube is read only where the device has one.
            if opcode == op::REQ_GET_TUBE_PARAM {
                let mut c = caps();
                c.features.push(feature("tube_preamp"));
                let t = MockTransport::new()
                    .reply(op::REQ_GET_ALL_PARAMS_CHUNK, Reply::Stall)
                    .reply(opcode, Reply::Disconnect)
                    .answering_everything(vec![0; 64]);
                s = Session::new(Box::new(t), c).unwrap();
            }
            assert!(capture(&mut s, None).is_err(), "0x{opcode:02X}");
        }
    }

    #[test]
    fn wp_tm21_a_refused_read_omits_the_key_and_keeps_the_rest() {
        let (mut s, _) = device(|m| {
            m.reply(op::REQ_GET_OUTPUT_GAIN, Reply::Stall)
                .reply(op::REQ_GET_MASTER_VOLUME, Reply::Stall)
                .reply(
                    op::REQ_GET_LOUDNESS_REF,
                    Reply::Error(TransportError::Denied("hub".into())),
                )
        });
        let doc = capture(&mut s, None).unwrap();
        let out = doc.channels.iter().find(|c| c.is_output).unwrap();
        assert_eq!(out.gain_db, None, "not 0 dB");
        assert_eq!(out.delay_ms, Some(-6.0), "the next read still counts");
        assert_eq!(doc.global.master_volume_db, None);
        assert_eq!(doc.global.user_volume_db, Some(-6.0));
        assert_eq!(doc.loudness.ref_spl, None);

        let v: serde_json::Value = serde_json::from_str(&write(&doc)).unwrap();
        assert!(v["global"].get("masterVolumeDb").is_none(), "{v}");
        assert!(v["global"].get("userVolumeDb").is_some(), "{v}");
        assert!(v["channels"][2].get("gainDb").is_none(), "{v}");
        // An input has no strip; these are there for the Console's shape.
        assert_eq!(v["channels"][0]["gainDb"], 0.0, "{v}");
        let left = omitted_settings(&doc);
        for key in [
            "global.masterVolumeDb",
            "channels.gainDb",
            "loudness.refSpl",
        ] {
            assert!(left.contains(&key.to_string()), "{key} in {left:?}");
        }
        assert!(!left.iter().any(|k| k == "global.userVolumeDb"), "{left:?}");
    }

    /// An image older than a field holds zeros there, which are no value: the
    /// device's wire format decides before the bytes are looked at.
    #[test]
    fn wp_tm21_an_image_holds_only_what_its_wire_format_carries() {
        let mut t = dspi_transport::FakeDevice::demo();
        let caps = crate::probe::probe(&mut t).unwrap();
        let st = DeviceState::new(caps, crate::read_bulk(&mut t).unwrap());
        for (path, since) in [
            ("lev.masks", 18),
            ("loud.mask", 19),
            ("cf.outputs", 20),
            ("in.i2s.clock", 21),
            ("bass.cutoff", 23),
            ("up.strength", 25),
            ("up.presence", 26),
            ("sub.low", 29),
            ("sub.top", 30),
            ("tube.drive", 31),
        ] {
            let at = |wire: u8| image_value(&st, wire_compat::features(wire), path, &[]);
            assert_eq!(at(since - 1), None, "{path} before V{since}");
            assert!(at(since).is_some(), "{path} from V{since}");
        }
        // These every mapped image holds.
        for path in [
            "vol.master",
            "vol.user",
            "bypass",
            "loud.on",
            "cf.on",
            "lev.on",
        ] {
            let got = image_value(&st, wire_compat::features(16), path, &[]);
            assert!(got.is_some(), "{path}");
        }
        let limiter = |wire: u8| image_value(&st, wire_compat::features(wire), "limit.on", &[0]);
        assert_eq!(limiter(31), None);
        assert!(limiter(32).is_some());
        // No image holds this one, at any format.
        assert_eq!(image_value(&st, feat::ALL, "vol.master.mode", &[]), None);
    }

    /// A read-only device with a refusing getter and a refusing bulk read has
    /// nothing to give, so the key is omitted rather than the export failed.
    #[test]
    fn wp_tm21_a_read_only_device_that_refuses_its_image_too_omits() {
        let mut c = caps();
        c.wire_format = 17;
        let t = MockTransport::new()
            .reply(op::REQ_GET_ALL_PARAMS_CHUNK, Reply::Stall)
            .reply(op::REQ_GET_MASTER_VOLUME, Reply::Stall)
            .answering_everything(vec![0; 64]);
        let mut s = Session::new(Box::new(t), c).unwrap();
        let doc = capture(&mut s, None).unwrap();
        assert_eq!(doc.global.master_volume_db, None);
        assert!(doc.matrix.is_empty());
    }

    /// A position in these lists is an input or a band, so nothing closes up.
    #[test]
    fn wp_tm21_a_refusal_never_shifts_a_list() {
        let (mut s, _) = device(|m| {
            m.reply(
                op::REQ_GET_PREAMP_CH,
                Reply::Sequence(vec![Reply::Data(vec![0; 4]), Reply::Stall]),
            )
        });
        assert_eq!(
            capture(&mut s, None).unwrap().global.input_preamps_db,
            [0.0]
        );

        // A bank is whole or absent: `apply` flattens what a bank leaves out.
        let (mut s, _) = device(|m| m.reply(op::REQ_GET_EQ_PARAM, Reply::Stall));
        let doc = capture(&mut s, None).unwrap();
        assert!(doc.channels.iter().all(|c| c.eq.is_empty()), "{doc:?}");
        assert!(omitted_settings(&doc).contains(&"channels.eq".to_string()));
    }

    /// A block with nothing in it is left out whole, which is what keeps Qt's
    /// importer from defaulting each of its keys.
    #[test]
    fn wp_tm21_a_block_with_nothing_in_it_is_left_out() {
        let (mut s, _) = device(|m| {
            [
                op::REQ_GET_LOUDNESS,
                op::REQ_GET_LOUDNESS_REF,
                op::REQ_GET_LOUDNESS_INTENSITY,
            ]
            .into_iter()
            .fold(m, |m, o| m.reply(o, Reply::Stall))
        });
        let doc = capture(&mut s, None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&write(&doc)).unwrap();
        assert!(v.get("loudness").is_none(), "{v}");
        assert!(omitted_settings(&doc).contains(&"loudness".to_string()));
        assert_eq!(
            parse(&write(&doc)).unwrap().loudness,
            LoudnessBlock::default()
        );
    }

    /// The Mac reads with `value(key, fallback)` and rejects only a document
    /// with no `schemaVersion` or `channels` (PresetDocument.swift:627, 729-744).
    /// However little the device reports, those two are there.
    #[test]
    fn wp_tm21_what_the_macs_decoder_requires_is_never_omitted() {
        // Nothing scripted is a stall, and over MIDI a stall is final, which
        // spares this test USB's retries.
        struct Midi(MockTransport);
        impl Transport for Midi {
            fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
                self.0.control_in(o, v, l)
            }
            fn control_in_upto(
                &mut self,
                o: u8,
                v: u16,
                l: u16,
            ) -> dspi_transport::Result<Vec<u8>> {
                self.0.control_in_upto(o, v, l)
            }
            fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
                self.0.control_out(o, v, d)
            }
            fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
                self.0.descriptor()
            }
            fn caps(&self) -> dspi_transport::TransportCaps {
                dspi_transport::TransportCaps {
                    kind: dspi_transport::TransportKind::Midi,
                    ..dspi_transport::TransportCaps::USB
                }
            }
        }
        let mut s = Session::new(Box::new(Midi(MockTransport::new())), caps()).unwrap();
        let doc = capture(&mut s, None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&write(&doc)).unwrap();
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["channels"].as_array().unwrap().len(), 5);
        assert!(v["meta"].get("wireFormatVersion").is_some());
        assert!(parse(&write(&doc)).is_ok());
    }

    #[test]
    fn wp_tm21_an_import_leaves_an_omitted_key_alone_and_says_so() {
        let doc = parse(
            r#"{"schemaVersion":1,
                "global":{"userVolumeDb":-3},
                "channels":[{"channelId":0,"isOutput":false,"eq":[{"type":1}]},
                            {"channelId":2,"outputIndex":0,"isOutput":true,"delayMs":5}]}"#,
        )
        .unwrap();
        let (mut s, log) = device(|m| m);
        let report = apply(
            &mut s,
            &doc,
            ApplyOptions {
                audio_processing: true,
                volume_levels: true,
                hardware_io: false,
            },
        );
        for opcode in [
            op::REQ_SET_OUTPUT_GAIN,
            op::REQ_SET_OUTPUT_MUTE,
            op::REQ_SET_OUTPUT_ENABLE,
            op::REQ_SET_OUTPUT_DELAY,
            op::REQ_SET_MASTER_VOLUME,
            op::REQ_SET_BYPASS,
            op::REQ_SET_INPUT_SOURCE,
            op::REQ_SET_LOUDNESS,
            op::REQ_SET_CROSSFEED,
            op::REQ_SET_LEVELLER_ENABLE,
            op::REQ_SET_PREAMP_CH,
        ] {
            assert_eq!(sets(&log, opcode), 0, "0x{opcode:02X}");
        }
        assert_eq!(sets(&log, op::REQ_SET_USER_VOLUME), 1, "this one it holds");
        assert_eq!(sets(&log, op::REQ_SET_DELAY), 1, "and this one");

        for key in [
            "channels.gainDb",
            "global.masterVolumeDb",
            "loudness",
            "matrix",
        ] {
            assert!(report.left_alone.contains(&key.to_string()), "{key}");
        }
        assert!(!report.is_clean());
        let lines = report.lines(false).join("\n");
        assert!(lines.contains("Left as they are"), "{lines}");
        assert!(lines.contains("channels.gainDb"), "{lines}");
    }

    /// What was not asked for is not reported as left out for lack of a value.
    #[test]
    fn wp_tm21_the_report_names_only_what_the_options_asked_for() {
        let doc = parse(r#"{"schemaVersion":1,"channels":[{"channelId":0}]}"#).unwrap();
        let (mut s, _) = device(|m| m);
        let report = apply(&mut s, &doc, ApplyOptions::default());
        assert!(!report.left_alone.iter().any(|k| k.contains("Volume")));
    }

    #[test]
    fn wp_tm21_documents_from_qt_and_the_mac_hold_every_key() {
        for (name, text) in [("qt", QT), ("mac", MAC)] {
            let doc = parse(text).unwrap();
            assert_eq!(omitted_settings(&doc), Vec::<String>::new(), "{name}");
            assert!(doc.global.master_volume_db.is_some(), "{name}");
            assert!(doc.channels.iter().all(|c| c.delay_ms.is_some()), "{name}");
        }
    }

    #[test]
    fn wp_tm21_an_old_wire_format_warns_on_import() {
        let doc_of = |wire: i32| {
            let mut d = parse(r#"{"schemaVersion":1,"channels":[{"channelId":0}]}"#).unwrap();
            d.meta.wire_format_version = wire;
            d
        };
        let (mut s, _) = device(|m| m);
        let report = apply(&mut s, &doc_of(17), ApplyOptions::default());
        let lines = report.lines(true).join("\n");
        assert!(
            lines.contains("Warning") && lines.contains("V17"),
            "{lines}"
        );

        let this_build = i32::from(WIRE_FORMAT_VERSION);
        for quiet in [0, this_build, this_build + 1] {
            let (mut s, _) = device(|m| m);
            let report = apply(&mut s, &doc_of(quiet), ApplyOptions::default());
            assert_eq!(report.warnings, Vec::<String>::new(), "V{quiet}");
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("wp_tm21_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wp_tm21_save_file_replaces_whole_and_leaves_nothing_behind() {
        let dir = scratch("save");
        let path = dir.join("a.dspipreset");
        save_file(&path, "first").unwrap();
        save_file(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let names = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(names, 1, "no temporary file left");

        // A target that cannot be replaced keeps what it held.
        let blocked = dir.join("dir");
        std::fs::create_dir(&blocked).unwrap();
        assert!(save_file(&blocked, "x").is_err());
        assert!(save_file(&dir.join("missing").join("x"), "x").is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn wp_tm21_save_file_keeps_the_mode_and_follows_a_link() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let real = dir.join("real");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        save_file(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wp_tm21_a_failed_read_fails_a_filter_capture() {
        for opcode in [op::REQ_GET_PREAMP_CH, op::REQ_GET_EQ_PARAM] {
            let (mut s, _) = device(|m| m.reply(opcode, Reply::Disconnect));
            let e = capture_filters(&mut s).expect_err("a lost device");
            assert!(matches!(e.source, TransportError::Disconnected), "{e}");
        }
        // Partway through a bank: the second band's third transfer.
        let mut replies = vec![Reply::Data(vec![0; 4]); 7];
        replies.push(Reply::Disconnect);
        let (mut s, _) = device(|m| m.reply(op::REQ_GET_EQ_PARAM, Reply::Sequence(replies)));
        assert!(capture_filters(&mut s).is_err());
    }

    /// The defect: a band that goes unreported used to close up the list, so the
    /// next band landed in its place on import.
    #[test]
    fn wp_tm21_a_refused_band_leaves_the_filter_bank_absent_not_shorter() {
        let mut replies = vec![Reply::Data(vec![0; 4]); 7];
        replies.push(Reply::Stall);
        let (mut s, log) = device(|m| m.reply(op::REQ_GET_EQ_PARAM, Reply::Sequence(replies)));
        let doc = capture_filters(&mut s).unwrap();
        assert!(doc.channels.iter().all(|c| c.peq.is_empty()), "{doc:?}");
        let left = doc.unreported();
        assert!(left.iter().any(|l| l.contains("EQ")), "{left:?}");

        // And an import leaves such a bank alone.
        let before = log.lock().unwrap().len();
        let mut report = FilterReport::default();
        apply_filter_channel(&mut s, 0, &doc.channels[0], &mut report);
        assert_eq!(report.bands_applied, 0);
        let sent = log.lock().unwrap()[before..]
            .iter()
            .filter(|e| e.opcode == op::REQ_SET_EQ_PARAM)
            .count();
        assert_eq!(sent, 0);
    }

    #[test]
    fn wp_tm21_a_refused_preamp_or_crossover_is_only_that_one_absent() {
        let (mut s, _) = device(|m| m.reply(op::REQ_GET_PREAMP_CH, Reply::Stall));
        let doc = capture_filters(&mut s).unwrap();
        let input = doc.channels.iter().find(|c| !c.is_output).unwrap();
        assert_eq!(input.preamp_db, None);
        assert_eq!(input.peq.len(), 2, "the bands are all there");
        let output = doc.channels.iter().find(|c| c.is_output).unwrap();
        assert_eq!(output.peq.len(), 2);
        assert_eq!(output.crossover.len(), CROSSOVER_BANDS as usize);
        assert_eq!(
            doc.unreported(),
            vec![
                format!("{}: preamp", doc.channels[0].header),
                format!("{}: preamp", doc.channels[1].header),
            ]
        );
    }

    #[test]
    fn wp_tm21_a_device_that_reports_everything_leaves_no_filters_out() {
        let (mut s, _) = device(|m| m);
        assert_eq!(
            capture_filters(&mut s).unwrap().unreported(),
            Vec::<String>::new()
        );
    }
}
