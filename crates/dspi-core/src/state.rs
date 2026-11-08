//! The mirror codec: the device's bulk image <-> [`DspState`].
//!
//! A port of the old desktop core's `protocol::decode_bulk` /
//! `encode_bulk` (the Android console's copy, which also carries the K340
//! crossfeed pair map), on `dspi-proto`'s generated offsets instead of
//! hand-copied ones. The contract the rest of the core relies on:
//!
//! - `decode_bulk(image)` fills every field the image carries and keeps the
//!   image itself in `bulk_raw` (`bulk_valid` true for an exact V32 image);
//! - `encode_bulk(state)` starts from `state.bulk_raw` and writes every
//!   decoded field back, so `decode(encode(s)) == s` for every field (the old
//!   core's `every_decoded_field_is_encoded` test) and bytes this build does
//!   not model are preserved;
//! - fields the image does not carry (preset directory and names, Core 1
//!   mode, DAC mute support, control interfaces, subharm solo) are left
//!   alone by `decode_bulk`.
//!
//! Three encodings are lossy on decode: names (UTF-8 is decoded lossily and
//! cut at the first NUL), filter bands (`bypass` is "byte == 1", the
//! Linkwitz target Q is clamped to 0.1..20 on encode) and the S/PDIF 2-4
//! enable byte. `encode_bulk` leaves the image's bytes for such a field alone
//! when they already decode to the state's value, so
//! `decode(encode(decode(x))) == decode(x)` holds for every V32 image `x`,
//! not just for well-formed ones (§1.9 property).
//!
//! The fixture `tests/fixtures/device/dspstate_v32.json` (the Android app's,
//! made by the old core from a real capture) is the expected decode of
//! `tests/fixtures/link/device/{platform,bulk_v32}.bin`.
//!
//! Owned by WP C0 (docs/core-merge/PLAN.md).

use dspi_proto::enums::FilterType;
use dspi_proto::generated::{self as g, tube};
use dspi_proto::value::decode_qp;
use dspi_proto::wire::{
    INPUT_CONFIG_FIELDS, LIMITER_OUTPUT_FIELDS, LIMITER_OUTPUT_LEN, SUBHARM_FIELDS, TUBE_FIELDS,
    WireError, decode_p1, encode_p1,
};

use crate::consts::*;
use crate::records::{CtrlIfaceStatus, DspState, FilterParams, I2cConfig, UartConfig};

/// The wire format this codec reads and writes.
const V32: u8 = g::wire::WIRE_FORMAT_VERSION as u8;
/// `sizeof(WireBulkParams)` at V32.
const BULK_SIZE: usize = g::BULK_SIZE;
const BULK_SIZE_U16: u16 = {
    assert!(BULK_SIZE <= u16::MAX as usize);
    BULK_SIZE as u16
};

// ── Layout ────────────────────────────────────────────────────────────
//
// Section offsets and lengths come from `dspi_proto::generated` (parsed from
// the vendored `bulk_params.h`). Field offsets inside a section come from
// the `wire::*_FIELDS` tables where dspi-proto has one; the others are the
// struct layouts in `bulk_params.h`, cited per section.

type Section = (usize, usize);
const S_HEADER: Section = (g::OFF_HEADER, g::LEN_HEADER);
const S_GLOBAL: Section = (g::OFF_GLOBAL, g::LEN_GLOBAL);
const S_CROSSFEED: Section = (g::OFF_CROSSFEED, g::LEN_CROSSFEED);
const S_DELAYS: Section = (g::OFF_DELAYS, g::LEN_DELAYS);
const S_CROSSPOINTS: Section = (g::OFF_CROSSPOINTS, g::LEN_CROSSPOINTS);
const S_OUTPUTS: Section = (g::OFF_OUTPUTS, g::LEN_OUTPUTS);
const S_PINS: Section = (g::OFF_PINS, g::LEN_PINS);
const S_EQ: Section = (g::OFF_EQ, g::LEN_EQ);
const S_CHANNEL_NAMES: Section = (g::OFF_CHANNEL_NAMES, g::LEN_CHANNEL_NAMES);
const S_I2S: Section = (g::OFF_I2S_CONFIG, g::LEN_I2S_CONFIG);
const S_LEVELLER: Section = (g::OFF_LEVELLER, g::LEN_LEVELLER);
const S_PREAMP: Section = (g::OFF_PREAMP, g::LEN_PREAMP);
const S_MASTER_VOLUME: Section = (g::OFF_MASTER_VOLUME, g::LEN_MASTER_VOLUME);
const S_INPUT_CONFIG: Section = (g::OFF_INPUT_CONFIG, g::LEN_INPUT_CONFIG);
const S_LG_SOUND_SYNC: Section = (g::OFF_LG_SOUND_SYNC, g::LEN_LG_SOUND_SYNC);
const S_USER_VOLUME: Section = (g::OFF_USER_VOLUME, g::LEN_USER_VOLUME);
const S_DAC_HW_MUTE: Section = (g::OFF_DAC_HW_MUTE, g::LEN_DAC_HW_MUTE);
const S_CROSSOVERS: Section = (g::OFF_CROSSOVERS, g::LEN_CROSSOVERS);
const S_ADAT: Section = (g::OFF_ADAT_CONFIG, g::LEN_ADAT_CONFIG);
const S_PSYBASS: Section = (g::OFF_PSYBASS, g::LEN_PSYBASS);
const S_UPMIX: Section = (g::OFF_UPMIX, g::LEN_UPMIX);
const S_SUBHARM: Section = (g::OFF_SUBHARM, g::LEN_SUBHARM);
const S_TUBE: Section = (g::OFF_TUBE, g::LEN_TUBE);
const S_LIMITER: Section = (g::OFF_LIMITER, g::LEN_LIMITER);

/// `WireHeader` (bulk_params.h section 1).
mod hdr {
    pub const VERSION: usize = 0;
    pub const PLATFORM: usize = 1;
    pub const NUM_CHANNELS: usize = 2;
    pub const NUM_OUTPUTS: usize = 3;
    pub const NUM_INPUTS: usize = 4;
    pub const PAYLOAD_LENGTH: usize = 6;
}

/// `WireGlobalParams` (section 2).
mod global {
    pub const PREAMP: usize = 0;
    pub const BYPASS: usize = 4;
    pub const LOUDNESS_ENABLED: usize = 5;
    pub const LOUDNESS_MASK: usize = 6;
    pub const LOUDNESS_REF: usize = 8;
    pub const LOUDNESS_INTENSITY: usize = 12;
}

/// `WireCrossfeedParams` (section 3).
mod xfeed {
    pub const ENABLED: usize = 0;
    pub const PRESET: usize = 1;
    pub const ITD: usize = 2;
    pub const PAIR_MASK: usize = 3;
    pub const FREQ: usize = 4;
    pub const FEED: usize = 8;
    pub const PAIR_MAP: usize = 12;
}

/// `WireCrosspoint`, 8 bytes, `[input][output]` row-major (section 6).
const CROSSPOINT_LEN: usize = 8;
const MATRIX_ROW_LEN: usize = CROSSPOINT_LEN * MAX_OUTPUTS;
/// `WireOutputChannel`, 12 bytes (section 7).
const OUTPUT_LEN: usize = 12;
/// `WireBandParams`: type, bypass, Linkwitz target Q (u16, Q x 512), then
/// freq, Q and gain as floats (sections 9 and 19).
const BAND_LEN: usize = 16;
const EQ_CHANNEL_LEN: usize = BAND_LEN * FIRMWARE_BANDS_PER_CHANNEL;
const XOVER_CHANNEL_LEN: usize = BAND_LEN * MAX_XOVER_BANDS;
/// A name holds at most this many bytes; the last byte is always NUL.
const NAME_MAX: usize = CHANNEL_NAME_LEN - 1;

/// `WireI2SConfig` (section 11).
mod i2s {
    pub const OUTPUT_TYPES: usize = 0;
    pub const BCK_PIN: usize = 4;
    pub const MCK_PIN: usize = 5;
    pub const MCK_ENABLED: usize = 6;
    pub const MCK_MULTIPLIER: usize = 7;
    pub const CLOCK_PIN_MODE_P1: usize = 8;
    pub const BCK_PIN_SLAVE: usize = 9;
}

/// `WireLevellerConfig` (section 12).
mod lev {
    pub const ENABLED: usize = 0;
    pub const SPEED: usize = 1;
    pub const LOOKAHEAD: usize = 2;
    pub const AMOUNT: usize = 4;
    pub const MAX_GAIN: usize = 8;
    pub const GATE: usize = 12;
    pub const DETECTOR_MASK: usize = 16;
    pub const APPLY_MASK: usize = 17;
}

/// `WireDacHwMute` (section 18).
mod dac {
    pub const ENABLED: usize = 0;
    pub const ACTIVE_LOW: usize = 1;
    pub const PIN: usize = 2;
    pub const HOLD_MS: usize = 4;
    pub const RELEASE_MS: usize = 6;
}

/// `WireAdatConfig` (section 20).
mod adat {
    pub const ENABLED: usize = 0;
    pub const PIN: usize = 1;
}

/// `WirePsybassParams` (section 21).
mod psy {
    pub const ENABLED: usize = 0;
    pub const MASK: usize = 2;
    pub const CUTOFF: usize = 4;
    pub const HARMONICS: usize = 8;
    pub const DRIVE: usize = 12;
    pub const CHARACTER: usize = 16;
    pub const ORIGINAL: usize = 20;
}

/// `WireInputConfig` (section 15), from `wire::INPUT_CONFIG_FIELDS`.
mod ic {
    use super::{INPUT_CONFIG_FIELDS as T, field};
    pub const INPUT_SOURCE: usize = field(&T, "input_source");
    pub const SPDIF_RX_PIN: usize = field(&T, "spdif_rx_pin");
    pub const I2S_RX_PIN: usize = field(&T, "i2s_rx_pin");
    pub const I2S_INPUT_RATE: usize = field(&T, "i2s_input_rate");
    pub const I2S_INPUT_CHANNELS: usize = field(&T, "i2s_input_channels");
    pub const I2S_RX_PIN_EXT: usize = field(&T, "i2s_rx_pin_ext");
    pub const SPDIF_RX_PIN_EXT: usize = field(&T, "spdif_rx_pin_ext");
    pub const SPDIF_RX_ENABLED_EXT_P1: usize = field(&T, "spdif_rx_enabled_ext_p1");
    pub const I2S_CLOCK_MODE: usize = field(&T, "i2s_clock_mode");
    pub const ADAT_INPUT_PIN: usize = field(&T, "adat_input_pin");
    pub const ADAT_INPUT_ENABLED_P1: usize = field(&T, "adat_input_enabled_p1");
    pub const ADAT_CLOCK_MODE_P1: usize = field(&T, "adat_clock_mode_p1");
}

/// `WireLimiterOutput` (section 25), from `wire::LIMITER_OUTPUT_FIELDS`.
mod lim {
    use super::{LIMITER_OUTPUT_FIELDS as T, field};
    pub const ENABLED: usize = field(&T, "enabled");
    pub const LINK_GROUP: usize = field(&T, "link_group");
    pub const THRESHOLD: usize = field(&T, "threshold_db");
    pub const RELEASE: usize = field(&T, "release_ms");
}

/// How a parameter-table entry (`DspState.upmix`, `subharm`, `tube`, all
/// `f32`) sits in the image.
#[derive(Clone, Copy)]
enum Kind {
    /// One byte, as a whole number, capped at the given maximum on decode.
    U8(u8),
    /// A little-endian `u16`, as a whole number.
    U16,
    /// A little-endian `f32`.
    F32,
    /// A signed byte in 0.5 dB steps, -12..+12 dB (the upmixer's presence).
    HalfDb,
}

/// `WireUpmixParams` (section 22): three mode bytes, the presence byte,
/// then ten floats in parameter-id order (strength .. decorrelation).
const UPMIX_MAP: [(u8, usize, Kind); UPMIX_PARAM_COUNT] = [
    (UPMIX_PARAM_ENABLED, 0, Kind::U8(u8::MAX)),
    (UPMIX_PARAM_CENTER_MODE, 1, Kind::U8(u8::MAX)),
    (UPMIX_PARAM_SURROUND_MODE, 2, Kind::U8(u8::MAX)),
    (UPMIX_PARAM_PRESENCE, 3, Kind::HalfDb),
    (UPMIX_PARAM_STRENGTH, 4, Kind::F32),
    (UPMIX_PARAM_WIDTH, 8, Kind::F32),
    (UPMIX_PARAM_THRESHOLD, 12, Kind::F32),
    (UPMIX_PARAM_ATTACK, 16, Kind::F32),
    (UPMIX_PARAM_RELEASE, 20, Kind::F32),
    (UPMIX_PARAM_DETECTOR_HPF, 24, Kind::F32),
    (UPMIX_PARAM_SURROUND_DELAY, 28, Kind::F32),
    (UPMIX_PARAM_SURROUND_HPF, 32, Kind::F32),
    (UPMIX_PARAM_SURROUND_LPF, 36, Kind::F32),
    (UPMIX_PARAM_DECORRELATION, 40, Kind::F32),
];

/// Offset of a `wire::SUBHARM_FIELDS` / `wire::TUBE_FIELDS` field.
const fn sh(name: &str) -> usize {
    field(&SUBHARM_FIELDS, name)
}
const fn tf(name: &str) -> usize {
    field(&TUBE_FIELDS, name)
}

/// `WireSubharmParams` (section 23), from `wire::SUBHARM_FIELDS`.
const SUBHARM_MAP: [(u8, usize, Kind); SUBHARM_PARAM_COUNT] = [
    (SUBHARM_PARAM_ENABLED, sh("enabled"), Kind::U8(u8::MAX)),
    (SUBHARM_PARAM_MASK, sh("output_mask"), Kind::U16),
    (SUBHARM_PARAM_LOW, sh("low_db"), Kind::F32),
    (SUBHARM_PARAM_HIGH, sh("high_db"), Kind::F32),
    (SUBHARM_PARAM_BOOST, sh("boost_db"), Kind::F32),
    (SUBHARM_PARAM_TOP, sh("top_db"), Kind::F32),
    (SUBHARM_PARAM_DEPTH, sh("select_depth"), Kind::F32),
    (SUBHARM_PARAM_HOLD, sh("select_hold_ms"), Kind::F32),
    (SUBHARM_PARAM_CEILING, sh("ceiling_db"), Kind::F32),
    (SUBHARM_PARAM_SELECT, sh("select_mode"), Kind::U8(u8::MAX)),
    (SUBHARM_PARAM_LINK, sh("link_pairs"), Kind::U8(u8::MAX)),
];

/// `WireTubeParams` (section 24), from `wire::TUBE_FIELDS`, indexed by the
/// firmware's `TUBE_PARAM_*`.
const TUBE_MAP: [(u16, usize, Kind); TUBE_PARAM_COUNT] = [
    (tube::TUBE_PARAM_ENABLED, tf("enabled"), Kind::U8(u8::MAX)),
    (
        tube::TUBE_PARAM_TUBE_TYPE,
        tf("tube_type"),
        Kind::U8(tube::TUBE_TYPE_MAX as u8),
    ),
    (
        tube::TUBE_PARAM_RECTIFIER,
        tf("rectifier"),
        Kind::U8(tube::TUBE_RECT_MAX as u8),
    ),
    (
        tube::TUBE_PARAM_XFMR_ENABLED,
        tf("xfmr_enabled"),
        Kind::U8(u8::MAX),
    ),
    (tube::TUBE_PARAM_OUTPUT_MASK, tf("output_mask"), Kind::U16),
    (tube::TUBE_PARAM_DRIVE_DB, tf("drive_db"), Kind::F32),
    (tube::TUBE_PARAM_BIAS_PCT, tf("bias_pct"), Kind::F32),
    (tube::TUBE_PARAM_ASYM_DB, tf("asym_db"), Kind::F32),
    (tube::TUBE_PARAM_HARDNESS_PCT, tf("hardness_pct"), Kind::F32),
    (tube::TUBE_PARAM_SAG_PCT, tf("sag_pct"), Kind::F32),
    (tube::TUBE_PARAM_XFMR_DAMPING, tf("xfmr_damping"), Kind::F32),
    (tube::TUBE_PARAM_XFMR_RES_HZ, tf("xfmr_res_hz"), Kind::F32),
    (tube::TUBE_PARAM_MIX_PCT, tf("mix_pct"), Kind::F32),
    (tube::TUBE_PARAM_TRIM_DB, tf("trim_db"), Kind::F32),
];

/// The offset of `name` in a dspi-proto `(name, offset, len)` field table.
// Called only from `const` items, so it runs at compile time: a name missing
// from the table, or an index past its end, fails the build instead of
// panicking at run time.
#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
const fn field(table: &[(&str, usize, usize)], name: &str) -> usize {
    let mut i = 0;
    while i < table.len() {
        let (n, off, _) = table[i];
        let (a, b) = (n.as_bytes(), name.as_bytes());
        if a.len() == b.len() {
            let mut k = 0;
            while k < a.len() && a[k] == b[k] {
                k += 1;
            }
            if k == a.len() {
                return off;
            }
        }
        i += 1;
    }
    panic!("field missing from the dspi-proto table")
}

// ── Defaults ──────────────────────────────────────────────────────────

/// `DspState.upmix` before any device (the old core's `UPMIX_DEFAULTS`).
const UPMIX_DEFAULTS: [f32; UPMIX_PARAM_COUNT] = [
    0.0, 1.0, 2.0, 100.0, 25.0, 30.0, 10.0, 100.0, 200.0, 12.0, 300.0, 7000.0, 90.0, 0.0,
];
/// `DspState.subharm` before any device (`SUBHARM_DEFAULTS`).
const SUBHARM_DEFAULTS: [f32; SUBHARM_PARAM_COUNT] = [
    0.0, 65535.0, 0.0, 0.0, -30.0, 0.0, 0.0, 100.0, 150.0, 0.0, 1.0,
];
/// `DspState.tube` before any device (`TUBE_DEFAULTS`).
const TUBE_DEFAULTS: [f32; TUBE_PARAM_COUNT] = [
    0.0, 65535.0, 1.0, -12.0, 10.0, 3.0, 40.0, 15.0, 1.0, 1.0, 2.0, 95.0, 100.0, 0.0,
];
const SPDIF_RX_PIN_DEFAULTS: [u8; 4] = [5, 20, 21, 22];
const I2S_RX_PIN_DEFAULTS: [u8; 4] = [1, 2, 3, 4];

fn default_band() -> FilterParams {
    FilterParams {
        filter_type: 0,
        bypass: false,
        freq: 1000.0,
        q: DEFAULT_Q,
        gain: 0.0,
        qp: DEFAULT_Q,
    }
}

/// The state before any device: what `get_state` returns while
/// disconnected (the old core's `DspState::default()`, including the
/// upmixer, subharm and tube default tables). Every vector has its full
/// length (`MAX_CHANNELS`, `MAX_OUTPUTS`, ...); `bulk_raw` is
/// `BULK_SIZE` zero bytes and `bulk_valid` is false.
pub fn default_state() -> DspState {
    DspState {
        platform_id: 0,
        num_channels: 7,
        num_input_channels: 2,
        num_output_channels: 5,
        format_version: 0,
        fw_major: 0,
        fw_minor: 0,
        fw_patch: 0,
        fw_beta: 0,
        compat: COMPAT_UNKNOWN,
        preamp_db: 0.0,
        bypass: false,
        input_preamp_db: vec![0.0; MAX_INPUTS],
        master_volume_db: -20.0,
        master_volume_mode: 0,
        user_volume_db: 0.0,
        user_mute: false,
        loudness_enabled: false,
        loudness_ref_spl: 83.0,
        loudness_intensity: 100.0,
        loudness_output_mask: 0,
        crossfeed_enabled: false,
        crossfeed_preset: 0,
        crossfeed_freq: 700.0,
        crossfeed_feed: 4.5,
        crossfeed_itd: true,
        crossfeed_output_pair_mask: 0,
        crossfeed_pair_map: vec![0; CROSSFEED_MAP_PAIRS],
        channel_delays: vec![0.0; MAX_CHANNELS],
        matrix_routing: vec![vec![false; MAX_OUTPUTS]; MAX_INPUTS],
        matrix_gain: vec![vec![0.0; MAX_OUTPUTS]; MAX_INPUTS],
        matrix_invert: vec![vec![false; MAX_OUTPUTS]; MAX_INPUTS],
        output_enabled: vec![false; MAX_OUTPUTS],
        output_muted: vec![false; MAX_OUTPUTS],
        output_gain_db: vec![0.0; MAX_OUTPUTS],
        output_delay_ms: vec![0.0; MAX_OUTPUTS],
        limiter_enabled: vec![false; MAX_OUTPUTS],
        limiter_link_group: vec![0; MAX_OUTPUTS],
        limiter_threshold_db: vec![0.0; MAX_OUTPUTS],
        limiter_release_ms: vec![100.0; MAX_OUTPUTS],
        num_pin_outputs: 3,
        output_pins: vec![6, 7, 10, 0, 0],
        output_config_mode: 1,
        filters: vec![vec![default_band(); BANDS_PER_CHANNEL]; MAX_CHANNELS],
        xover: vec![vec![default_band(); MAX_XOVER_BANDS]; MAX_CHANNELS],
        channel_names: vec![String::new(); MAX_CHANNELS],
        leveller_enabled: false,
        leveller_speed: 1,
        leveller_lookahead: false,
        leveller_amount: 50.0,
        leveller_max_gain_db: 12.0,
        leveller_gate_db: -60.0,
        leveller_detector_mask: 0x03,
        leveller_apply_mask: 0x03,
        input_source: 0,
        lg_sound_sync_enabled: false,
        spdif_inputs_enabled: 0x01,
        adat_input_enabled: false,
        spdif_rx_pins: SPDIF_RX_PIN_DEFAULTS.to_vec(),
        i2s_rx_pins: I2S_RX_PIN_DEFAULTS.to_vec(),
        i2s_input_channels: 2,
        i2s_input_rate: 1,
        i2s_clock_mode: 0,
        adat_input_pin: 0xFF,
        adat_input_clock_mode: 0,
        psybass_enabled: false,
        psybass_output_mask: 0,
        psybass_cutoff_hz: 80.0,
        psybass_harmonics_db: 0.0,
        psybass_drive_db: 6.0,
        psybass_character_pct: 50.0,
        psybass_original_db: 0.0,
        upmix: UPMIX_DEFAULTS.to_vec(),
        subharm: SUBHARM_DEFAULTS.to_vec(),
        subharm_solo: false,
        tube: TUBE_DEFAULTS.to_vec(),
        output_types: vec![0; 4],
        i2s_bck_pin: 14,
        i2s_bck_pin_slave: 26,
        i2s_clock_pin_mode: 0,
        mck_enabled: false,
        mck_pin: 13,
        mck_multiplier: 0,
        adat_out_enabled: false,
        adat_out_pin: 12,
        dac_mute_supported: false,
        dac_mute_enabled: false,
        dac_mute_active_low: true,
        dac_mute_pin: 11,
        dac_mute_hold_ms: 5,
        dac_mute_release_ms: 0,
        ctrl_iface_supported: false,
        uart: UartConfig {
            enabled: false,
            tx_pin: 16,
            rx_pin: 17,
            notify: false,
            baud: 115_200,
        },
        i2c: I2cConfig {
            enabled: false,
            sda_pin: 18,
            scl_pin: 19,
            address: 0x42,
        },
        ctrl_status: CtrlIfaceStatus::default(),
        core1_mode: 0,
        preset_occupied: 0,
        preset_names: vec![String::new(); MAX_PRESETS],
        active_preset_slot: 0,
        preset_startup_mode: 0,
        preset_default_slot: 0,
        bulk_valid: false,
        bulk_raw: vec![0; BULK_SIZE],
        // -- WP CG32 --
        i2s_clock_pin_mode_present: false,
        spdif_inputs_enabled_present: false,
        adat_input_enabled_present: false,
        adat_input_clock_mode_present: false,
    }
}

/// Longest delay (ms) the firmware's delay line holds for this state's
/// platform: 42 on RP2350 (`platform_id` 1, 2048 samples at 48 kHz), 21
/// otherwise (RP2040, 1024 samples). -- WP CG37 -- read from
/// [`dspi_proto::Platform::max_delay_ms`], the session's source too.
pub(crate) fn max_delay_ms_for(state: &DspState) -> f32 {
    dspi_proto::Platform::from_id(state.platform_id).max_delay_ms()
}

// ── REQ_GET_PLATFORM ──────────────────────────────────────────────────

/// Apply a `REQ_GET_PLATFORM` reply (4, 6 or 7 bytes) to `state`: platform
/// id, output count (capped at `MAX_OUTPUTS`) and firmware version.
///
/// Firmware before 1.1.6 sends 4 bytes with minor and patch packed into
/// nibbles; 1.1.6 and later append full-width minor, patch and the beta
/// ordinal. A reply without the ordinal that claims 1.1.6 or later is one of
/// the early 1.1.6 betas: `fw_beta` becomes [`FW_BETA_EARLY`]. Bytes past
/// the seventh are ignored. `compat` is the caller's to set.
///
/// Fails with `WireError::WrongSize` (state untouched) if the reply is
/// shorter than 4 bytes.
pub fn apply_platform(reply: &[u8], state: &mut DspState) -> Result<(), WireError> {
    let Some(&[platform, major, packed, outputs]) = reply.first_chunk::<4>() else {
        return Err(WireError::WrongSize {
            got: reply.len(),
            expected: 4,
        });
    };
    let (minor, patch) = match reply.get(4..6) {
        Some(&[minor, patch]) => (minor, patch),
        _ => (packed >> 4, packed & 0x0F),
    };
    let beta = match reply.get(6) {
        Some(&beta) => beta,
        None if (major, minor, patch) >= (1, 1, 6) => FW_BETA_EARLY,
        None => 0,
    };
    let s = state;
    s.platform_id = platform;
    s.num_output_channels = outputs.min(MAX_OUTPUTS as u8);
    s.fw_major = major;
    s.fw_minor = minor;
    s.fw_patch = patch;
    s.fw_beta = beta;
    Ok(())
}

// ── Byte access ───────────────────────────────────────────────────────
//
// Bounds-checked readers and writers. `decode_bulk` checks the image length
// first and `encode_bulk` always works on a `BULK_SIZE` buffer, so every
// access below is in range; the checks only make that impossible to get
// wrong. An out-of-range read gives 0, an out-of-range write does nothing.

#[derive(Clone, Copy)]
struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn sec(self, (off, len): Section) -> Rd<'a> {
        Rd(self.0.get(off..).and_then(|s| s.get(..len)).unwrap_or(&[]))
    }
    fn arr<const N: usize>(self, at: usize) -> Option<[u8; N]> {
        self.0.get(at..)?.first_chunk::<N>().copied()
    }
    fn u8(self, at: usize) -> u8 {
        self.0.get(at).copied().unwrap_or(0)
    }
    fn flag(self, at: usize) -> bool {
        self.u8(at) != 0
    }
    fn u16(self, at: usize) -> u16 {
        self.arr(at).map_or(0, u16::from_le_bytes)
    }
    fn f32(self, at: usize) -> f32 {
        self.arr(at).map_or(0.0, f32::from_le_bytes)
    }
    fn records(self, len: usize) -> impl Iterator<Item = Rd<'a>> {
        self.0.chunks_exact(len).map(Rd)
    }
    fn kind(self, at: usize, kind: Kind) -> f32 {
        match kind {
            Kind::U8(max) => f32::from(self.u8(at).min(max)),
            Kind::U16 => f32::from(self.u16(at)),
            Kind::F32 => self.f32(at),
            Kind::HalfDb => f32::from(i8::from_le_bytes([self.u8(at)]).clamp(-24, 24)) * 0.5,
        }
    }
}

struct Wr<'a>(&'a mut [u8]);

impl<'a> Wr<'a> {
    fn sec(&mut self, (off, len): Section) -> Wr<'_> {
        Wr(self
            .0
            .get_mut(off..)
            .and_then(|s| s.get_mut(..len))
            .unwrap_or(&mut []))
    }
    fn rd(&self) -> Rd<'_> {
        Rd(self.0)
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        if let Some(dst) = self.0.get_mut(at..).and_then(|s| s.get_mut(..bytes.len())) {
            dst.copy_from_slice(bytes);
        }
    }
    fn u8(&mut self, at: usize, v: u8) {
        self.put(at, &[v]);
    }
    fn flag(&mut self, at: usize, v: bool) {
        self.u8(at, u8::from(v));
    }
    fn u16(&mut self, at: usize, v: u16) {
        self.put(at, &v.to_le_bytes());
    }
    fn f32(&mut self, at: usize, v: f32) {
        self.put(at, &v.to_le_bytes());
    }
    fn records(self, len: usize) -> impl Iterator<Item = Wr<'a>> {
        self.0.chunks_exact_mut(len).map(Wr)
    }
    // `as` saturates float -> integer (NaN -> 0), the old core's behaviour.
    fn kind(&mut self, at: usize, kind: Kind, v: f32) {
        match kind {
            Kind::U8(_) => self.u8(at, v as u8),
            Kind::U16 => self.u16(at, v as u16),
            Kind::F32 => self.f32(at, v),
            Kind::HalfDb => {
                let half_db = (v * 2.0).round().clamp(-24.0, 24.0) as i8;
                self.put(at, &half_db.to_le_bytes());
            }
        }
    }
}

// ── Lossy fields ──────────────────────────────────────────────────────

fn parse_band(b: Rd<'_>) -> FilterParams {
    let filter_type = b.u8(0);
    FilterParams {
        filter_type,
        // The firmware's rule is "1 means bypassed", not "non-zero".
        bypass: b.u8(1) == 1,
        freq: b.f32(4),
        q: b.f32(8),
        gain: b.f32(12),
        // The reserved pair carries the Linkwitz target Q and nothing else.
        qp: if FilterType::from_raw(filter_type).is_linkwitz() {
            decode_qp(b.u16(2))
        } else {
            DEFAULT_Q
        },
    }
}

/// Bit-exact equality, so a band holding a NaN still counts as unchanged.
fn same_band(a: &FilterParams, b: &FilterParams) -> bool {
    a.filter_type == b.filter_type
        && a.bypass == b.bypass
        && a.freq.to_bits() == b.freq.to_bits()
        && a.q.to_bits() == b.q.to_bits()
        && a.gain.to_bits() == b.gain.to_bits()
        && a.qp.to_bits() == b.qp.to_bits()
}

/// Write band `p`, unless the bytes there already decode to it.
fn write_band(mut w: Wr<'_>, p: &FilterParams) {
    if same_band(&parse_band(w.rd()), p) {
        return;
    }
    // Wire encoding of the LT target Q: round(qp x 512), 0 for other types.
    let qp_x512 = if FilterType::from_raw(p.filter_type).is_linkwitz() {
        (p.qp.clamp(0.1, 20.0) * 512.0).round() as u16
    } else {
        0
    };
    w.u8(0, p.filter_type);
    w.flag(1, p.bypass);
    w.u16(2, qp_x512);
    w.f32(4, p.freq);
    w.f32(8, p.q);
    w.f32(12, p.gain);
}

/// A NUL-padded name: the bytes before the first NUL in the first
/// `NAME_MAX` bytes, UTF-8 decoded lossily.
fn parse_name(b: Rd<'_>) -> String {
    let b = b.0.get(..NAME_MAX).unwrap_or(b.0);
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(b.get(..end).unwrap_or_default()).into_owned()
}

/// Write `name`, cut to `NAME_MAX` bytes on a character boundary and
/// NUL-padded, unless the bytes there already read as `name`.
fn write_name(mut w: Wr<'_>, name: &str) {
    if parse_name(w.rd()) == name {
        return;
    }
    let len = (0..=name.len().min(NAME_MAX))
        .rev()
        .find(|&l| name.is_char_boundary(l))
        .unwrap_or(0);
    w.put(0, &[0; CHANNEL_NAME_LEN]);
    w.put(0, name.as_bytes().get(..len).unwrap_or_default());
}

/// `spdif_inputs_enabled` from `spdif_rx_enabled_ext_p1`: bit 0 (input 1)
/// is always set; bits 1-3 are the +1-encoded mask of inputs 2-4 (0 =
/// absent, read as "only input 1"). Bits 3-7 of a corrupt byte are
/// ignored.
///
/// So bit 0 of `DspState.spdif_inputs_enabled` is always set after a
/// decode, and a setter must keep it set: a state holding 0x02 encodes the
/// mask of inputs 2-4 only and re-decodes as 0x03.
fn parse_spdif_enabled(ext_p1: u8) -> u8 {
    0x01 | decode_p1(ext_p1).map_or(0, |m| (m & 0x07).wrapping_shl(1))
}

// -- WP CG32 --
/// Write one +1-encoded byte (0 = absent) at `at`: `wire + 1` when the
/// state has the value, 0 when it does not. The state has it when its
/// presence flag says so, or when the value is not what an absent byte reads
/// as (`moved`: a setter changed it). `state` is the value and the flag;
/// `read` is how decoding reads a byte. Nothing is written when the image's
/// byte already reads as the state does, so an out-of-range byte stays.
fn write_p1<T: PartialEq>(
    w: &mut Wr<'_>,
    at: usize,
    state: (T, bool),
    moved: bool,
    wire: u8,
    read: impl Fn(u8) -> T,
) {
    let cur = w.rd().u8(at);
    let present = state.1 || moved;
    if (read(cur), cur != 0) != (state.0, present) {
        w.u8(at, if present { encode_p1(Some(wire)) } else { 0 });
    }
}

// ── The codec ─────────────────────────────────────────────────────────

/// Decode a V32-layout image into `state` (see the module docs). An image of
/// another wire format is normalised first by the caller
/// (`dspi_proto::wire_compat`), which also decides `bulk_valid`.
///
/// Sets `format_version` and the channel counts from the header (capped at
/// `MAX_*`), every image-carried field, `bulk_raw` (a copy of `image`) and
/// `bulk_valid` = true; a caller decoding a normalised (read-only) image
/// clears `bulk_valid` afterwards. The fields the image marks absent (an
/// unset I2S or S/PDIF pin, the I2S slave BCK pin, the ADAT output pin and
/// input clock mode, the I2S channel count) keep their value in `state`.
/// The four `*_present` flags say whether the image carried the I2S
/// clock-pin mode, the S/PDIF 2-4 mask, the ADAT input enable and clock mode
/// (their +1-encoded bytes are not 0); `encode_bulk` writes them back absent
/// unless the flag is set or the value was changed.
///
/// Fails, with `state` untouched, with `WireError::WrongSize` unless the
/// image is exactly `BULK_SIZE` (6136) bytes, and with
/// `WireError::UnsupportedVersion` unless its header says V32.
pub fn decode_bulk(image: &[u8], state: &mut DspState) -> Result<(), WireError> {
    if image.len() != BULK_SIZE {
        return Err(WireError::WrongSize {
            got: image.len(),
            expected: BULK_SIZE,
        });
    }
    let d = Rd(image);
    let h = d.sec(S_HEADER);
    let version = h.u8(hdr::VERSION);
    if version != V32 {
        return Err(WireError::UnsupportedVersion {
            got: version,
            expected: V32,
        });
    }
    let s = state;

    // Header
    s.format_version = version;
    s.platform_id = h.u8(hdr::PLATFORM);
    s.num_channels = h.u8(hdr::NUM_CHANNELS).min(MAX_CHANNELS as u8);
    s.num_output_channels = h.u8(hdr::NUM_OUTPUTS).min(MAX_OUTPUTS as u8);
    s.num_input_channels = h.u8(hdr::NUM_INPUTS).min(MAX_INPUTS as u8);

    // Global
    let gl = d.sec(S_GLOBAL);
    s.preamp_db = gl.f32(global::PREAMP);
    s.bypass = gl.flag(global::BYPASS);
    s.loudness_enabled = gl.flag(global::LOUDNESS_ENABLED);
    s.loudness_output_mask = gl.u16(global::LOUDNESS_MASK);
    s.loudness_ref_spl = gl.f32(global::LOUDNESS_REF);
    s.loudness_intensity = gl.f32(global::LOUDNESS_INTENSITY);

    // Crossfeed
    let cf = d.sec(S_CROSSFEED);
    s.crossfeed_enabled = cf.flag(xfeed::ENABLED);
    s.crossfeed_preset = cf.u8(xfeed::PRESET);
    s.crossfeed_itd = cf.flag(xfeed::ITD);
    s.crossfeed_output_pair_mask = cf.u8(xfeed::PAIR_MASK);
    s.crossfeed_freq = cf.f32(xfeed::FREQ);
    s.crossfeed_feed = cf.f32(xfeed::FEED);
    s.crossfeed_pair_map = cf
        .arr::<CROSSFEED_MAP_PAIRS>(xfeed::PAIR_MAP)
        .unwrap_or_default()
        .to_vec();

    // Delays (wire channel)
    s.channel_delays = d.sec(S_DELAYS).records(4).map(|r| r.f32(0)).collect();

    // Matrix [input][output]
    let rows = || d.sec(S_CROSSPOINTS).records(MATRIX_ROW_LEN);
    s.matrix_routing = rows()
        .map(|r| r.records(CROSSPOINT_LEN).map(|c| c.flag(0)).collect())
        .collect();
    s.matrix_invert = rows()
        .map(|r| r.records(CROSSPOINT_LEN).map(|c| c.flag(1)).collect())
        .collect();
    s.matrix_gain = rows()
        .map(|r| r.records(CROSSPOINT_LEN).map(|c| c.f32(4)).collect())
        .collect();

    // Outputs
    let outs = || d.sec(S_OUTPUTS).records(OUTPUT_LEN);
    s.output_enabled = outs().map(|o| o.flag(0)).collect();
    s.output_muted = outs().map(|o| o.flag(1)).collect();
    s.output_gain_db = outs().map(|o| o.f32(4)).collect();
    s.output_delay_ms = outs().map(|o| o.f32(8)).collect();

    // Pins
    let pins = d.sec(S_PINS);
    s.num_pin_outputs = pins.u8(0).min(MAX_PHYSICAL_OUTPUTS as u8);
    s.output_pins = pins
        .arr::<MAX_PHYSICAL_OUTPUTS>(1)
        .unwrap_or_default()
        .to_vec();

    // EQ [wire channel][band]: the first BANDS_PER_CHANNEL of each row
    s.filters = d
        .sec(S_EQ)
        .records(EQ_CHANNEL_LEN)
        .map(|ch| {
            ch.records(BAND_LEN)
                .take(BANDS_PER_CHANNEL)
                .map(parse_band)
                .collect()
        })
        .collect();

    // Channel names
    s.channel_names = d
        .sec(S_CHANNEL_NAMES)
        .records(CHANNEL_NAME_LEN)
        .map(parse_name)
        .collect();

    // Leveller
    let lv = d.sec(S_LEVELLER);
    s.leveller_enabled = lv.flag(lev::ENABLED);
    s.leveller_speed = lv.u8(lev::SPEED);
    s.leveller_lookahead = lv.flag(lev::LOOKAHEAD);
    s.leveller_amount = lv.f32(lev::AMOUNT);
    s.leveller_max_gain_db = lv.f32(lev::MAX_GAIN);
    s.leveller_gate_db = lv.f32(lev::GATE);
    s.leveller_detector_mask = lv.u8(lev::DETECTOR_MASK);
    s.leveller_apply_mask = lv.u8(lev::APPLY_MASK);

    // Per-input preamp, master volume
    s.input_preamp_db = d.sec(S_PREAMP).records(4).map(|r| r.f32(0)).collect();
    s.master_volume_db = d.sec(S_MASTER_VOLUME).f32(0);

    // Input config. Extension bytes use 0 for "absent, keep".
    let c = d.sec(S_INPUT_CONFIG);
    s.input_source = c.u8(ic::INPUT_SOURCE);
    // -- WP CG32 -- and the presence flags say which were there.
    let spdif_p1 = c.u8(ic::SPDIF_RX_ENABLED_EXT_P1);
    s.spdif_inputs_enabled = parse_spdif_enabled(spdif_p1);
    s.spdif_inputs_enabled_present = spdif_p1 != 0;
    let adat_p1 = c.u8(ic::ADAT_INPUT_ENABLED_P1);
    s.adat_input_enabled = adat_p1 == 2;
    s.adat_input_enabled_present = adat_p1 != 0;
    fit(&mut s.spdif_rx_pins, &SPDIF_RX_PIN_DEFAULTS);
    fit(&mut s.i2s_rx_pins, &I2S_RX_PIN_DEFAULTS);
    let ext = |at| c.arr::<3>(at).unwrap_or_default();
    keep_or_set(
        &mut s.spdif_rx_pins,
        c.u8(ic::SPDIF_RX_PIN),
        ext(ic::SPDIF_RX_PIN_EXT),
    );
    keep_or_set(
        &mut s.i2s_rx_pins,
        c.u8(ic::I2S_RX_PIN),
        ext(ic::I2S_RX_PIN_EXT),
    );
    s.i2s_input_rate = c.u8(ic::I2S_INPUT_RATE).min(2);
    if c.u8(ic::I2S_INPUT_CHANNELS) != 0 {
        s.i2s_input_channels = c.u8(ic::I2S_INPUT_CHANNELS);
    }
    s.i2s_clock_mode = c.u8(ic::I2S_CLOCK_MODE).min(1);
    // 0 on the wire is "unset", 0xFF in the state: the firmware's own
    // mapping (bulk_params.c:252), not a host convention. Keep it.
    s.adat_input_pin = match c.u8(ic::ADAT_INPUT_PIN) {
        0 => 0xFF,
        pin => pin,
    };
    let clock_p1 = c.u8(ic::ADAT_CLOCK_MODE_P1);
    if let Some(mode) = decode_p1(clock_p1) {
        s.adat_input_clock_mode = mode.min(1);
    }
    s.adat_input_clock_mode_present = clock_p1 != 0;

    // Psychoacoustic bass
    let p = d.sec(S_PSYBASS);
    s.psybass_enabled = p.flag(psy::ENABLED);
    s.psybass_output_mask = p.u16(psy::MASK);
    s.psybass_cutoff_hz = p.f32(psy::CUTOFF);
    s.psybass_harmonics_db = p.f32(psy::HARMONICS);
    s.psybass_drive_db = p.f32(psy::DRIVE);
    s.psybass_character_pct = p.f32(psy::CHARACTER);
    s.psybass_original_db = p.f32(psy::ORIGINAL);

    // Upmixer (all zero on RP2040), subharmonic synth, tube modeller
    s.upmix = decode_params(d.sec(S_UPMIX), &UPMIX_MAP, UPMIX_PARAM_COUNT);
    s.subharm = decode_params(d.sec(S_SUBHARM), &SUBHARM_MAP, SUBHARM_PARAM_COUNT);
    s.tube = decode_params(d.sec(S_TUBE), &TUBE_MAP, TUBE_PARAM_COUNT);

    // LG Sound Sync (only `enabled` is honoured on SET)
    s.lg_sound_sync_enabled = d.sec(S_LG_SOUND_SYNC).flag(0);

    // Output types and clocks
    let i = d.sec(S_I2S);
    s.output_types = i.arr::<4>(i2s::OUTPUT_TYPES).unwrap_or_default().to_vec();
    s.i2s_bck_pin = i.u8(i2s::BCK_PIN);
    s.mck_pin = i.u8(i2s::MCK_PIN);
    s.mck_enabled = i.flag(i2s::MCK_ENABLED);
    s.mck_multiplier = i.u8(i2s::MCK_MULTIPLIER).min(1);
    // +1 encoded, 0 = absent (read as unified)
    let pins_p1 = i.u8(i2s::CLOCK_PIN_MODE_P1);
    s.i2s_clock_pin_mode = pins_p1.saturating_sub(1).min(1);
    s.i2s_clock_pin_mode_present = pins_p1 != 0;
    if i.u8(i2s::BCK_PIN_SLAVE) != 0 {
        s.i2s_bck_pin_slave = i.u8(i2s::BCK_PIN_SLAVE);
    }
    let a = d.sec(S_ADAT);
    s.adat_out_enabled = a.flag(adat::ENABLED);
    if a.u8(adat::PIN) != 0 {
        s.adat_out_pin = a.u8(adat::PIN);
    }

    // DAC hardware mute
    let m = d.sec(S_DAC_HW_MUTE);
    s.dac_mute_enabled = m.flag(dac::ENABLED);
    s.dac_mute_active_low = m.flag(dac::ACTIVE_LOW);
    s.dac_mute_pin = m.u8(dac::PIN);
    s.dac_mute_hold_ms = m.u16(dac::HOLD_MS);
    s.dac_mute_release_ms = m.u16(dac::RELEASE_MS);

    // User volume / mute
    let u = d.sec(S_USER_VOLUME);
    s.user_volume_db = u.f32(0);
    s.user_mute = u.flag(4);

    // Crossover bands [wire channel][band]
    s.xover = d
        .sec(S_CROSSOVERS)
        .records(XOVER_CHANNEL_LEN)
        .map(|ch| ch.records(BAND_LEN).map(parse_band).collect())
        .collect();

    // Output limiter
    let lims = || d.sec(S_LIMITER).records(LIMITER_OUTPUT_LEN);
    s.limiter_enabled = lims().map(|l| l.flag(lim::ENABLED)).collect();
    s.limiter_link_group = lims().map(|l| l.u8(lim::LINK_GROUP)).collect();
    s.limiter_threshold_db = lims().map(|l| l.f32(lim::THRESHOLD)).collect();
    s.limiter_release_ms = lims().map(|l| l.f32(lim::RELEASE)).collect();

    s.bulk_raw = image.to_vec();
    s.bulk_valid = true;
    Ok(())
}

/// Resize `v` to the defaults' length, keeping what it has.
fn fit(v: &mut Vec<u8>, defaults: &[u8]) {
    if v.len() != defaults.len() {
        let mut n = defaults.to_vec();
        n.iter_mut().zip(v.iter()).for_each(|(a, b)| *a = *b);
        *v = n;
    }
}

/// Pin 0 is always set; pins 1..=3 only where the image has a non-zero pin.
fn keep_or_set(pins: &mut [u8], first: u8, ext: [u8; 3]) {
    let mut it = pins.iter_mut();
    if let Some(p) = it.next() {
        *p = first;
    }
    for (p, raw) in it.zip(ext) {
        if raw != 0 {
            *p = raw;
        }
    }
}

fn decode_params<I: Copy + Into<usize>>(
    sec: Rd<'_>,
    map: &[(I, usize, Kind)],
    n: usize,
) -> Vec<f32> {
    let mut v = vec![0.0; n];
    for &(id, at, kind) in map {
        if let Some(slot) = v.get_mut(id.into()) {
            *slot = sec.kind(at, kind);
        }
    }
    v
}

fn encode_params<I: Copy + Into<usize>>(mut sec: Wr<'_>, map: &[(I, usize, Kind)], v: &[f32]) {
    for &(id, at, kind) in map {
        if let Some(&x) = v.get(id.into()) {
            sec.kind(at, kind, x);
        }
    }
}

/// The image for `state`: `state.bulk_raw` with every decoded field written
/// back, and a V32 header (`format_version` 32, `payload_length` 6136).
///
/// Always `BULK_SIZE` bytes: a `bulk_raw` of another length is cut or
/// zero-padded first. Vectors shorter than their full length leave the
/// image's bytes for the missing entries alone. The header's platform and
/// channel counts are the image's, never the state's. Never fails.
///
/// `spdif_inputs_enabled` bit 0 (S/PDIF input 1) is not on the wire: only
/// bits 1-3 are encoded, and decoding always sets bit 0. A setter must
/// keep it set (0x02 would come back as 0x03).
pub fn encode_bulk(state: &DspState) -> Vec<u8> {
    let s = state;
    let mut img = vec![0u8; BULK_SIZE];
    img.iter_mut().zip(&s.bulk_raw).for_each(|(d, r)| *d = *r);
    let mut d = Wr(&mut img);

    {
        let mut h = d.sec(S_HEADER);
        h.u8(hdr::VERSION, V32);
        h.u16(hdr::PAYLOAD_LENGTH, BULK_SIZE_U16);
    }
    {
        let mut w = d.sec(S_GLOBAL);
        w.f32(global::PREAMP, s.preamp_db);
        w.flag(global::BYPASS, s.bypass);
        w.flag(global::LOUDNESS_ENABLED, s.loudness_enabled);
        w.u16(global::LOUDNESS_MASK, s.loudness_output_mask);
        w.f32(global::LOUDNESS_REF, s.loudness_ref_spl);
        w.f32(global::LOUDNESS_INTENSITY, s.loudness_intensity);
    }
    {
        let mut w = d.sec(S_CROSSFEED);
        w.flag(xfeed::ENABLED, s.crossfeed_enabled);
        w.u8(xfeed::PRESET, s.crossfeed_preset);
        w.flag(xfeed::ITD, s.crossfeed_itd);
        w.u8(xfeed::PAIR_MASK, s.crossfeed_output_pair_mask);
        w.f32(xfeed::FREQ, s.crossfeed_freq);
        w.f32(xfeed::FEED, s.crossfeed_feed);
        if let Some(map) = s.crossfeed_pair_map.get(..CROSSFEED_MAP_PAIRS) {
            w.put(xfeed::PAIR_MAP, map);
        }
    }
    for (mut w, v) in d.sec(S_DELAYS).records(4).zip(&s.channel_delays) {
        w.f32(0, *v);
    }
    let matrix = s
        .matrix_routing
        .iter()
        .zip(&s.matrix_invert)
        .zip(&s.matrix_gain);
    for (row, ((routing, invert), gain)) in d.sec(S_CROSSPOINTS).records(MATRIX_ROW_LEN).zip(matrix)
    {
        let cells = routing.iter().zip(invert).zip(gain);
        for (mut c, ((r, i), g)) in row.records(CROSSPOINT_LEN).zip(cells) {
            c.flag(0, *r);
            c.flag(1, *i);
            c.f32(4, *g);
        }
    }
    let outs = s
        .output_enabled
        .iter()
        .zip(&s.output_muted)
        .zip(&s.output_gain_db)
        .zip(&s.output_delay_ms);
    for (mut w, (((en, mu), gain), delay)) in d.sec(S_OUTPUTS).records(OUTPUT_LEN).zip(outs) {
        w.flag(0, *en);
        w.flag(1, *mu);
        w.f32(4, *gain);
        w.f32(8, *delay);
    }
    {
        let mut w = d.sec(S_PINS);
        w.u8(0, s.num_pin_outputs);
        if let Some(pins) = s.output_pins.get(..MAX_PHYSICAL_OUTPUTS) {
            w.put(1, pins);
        }
    }
    for (ch, bands) in d.sec(S_EQ).records(EQ_CHANNEL_LEN).zip(&s.filters) {
        for (w, band) in ch
            .records(BAND_LEN)
            .zip(bands.iter().take(BANDS_PER_CHANNEL))
        {
            write_band(w, band);
        }
    }
    for (w, name) in d
        .sec(S_CHANNEL_NAMES)
        .records(CHANNEL_NAME_LEN)
        .zip(&s.channel_names)
    {
        write_name(w, name);
    }
    {
        let mut w = d.sec(S_LEVELLER);
        w.flag(lev::ENABLED, s.leveller_enabled);
        w.u8(lev::SPEED, s.leveller_speed);
        w.flag(lev::LOOKAHEAD, s.leveller_lookahead);
        w.f32(lev::AMOUNT, s.leveller_amount);
        w.f32(lev::MAX_GAIN, s.leveller_max_gain_db);
        w.f32(lev::GATE, s.leveller_gate_db);
        w.u8(lev::DETECTOR_MASK, s.leveller_detector_mask);
        w.u8(lev::APPLY_MASK, s.leveller_apply_mask);
    }
    for (mut w, v) in d.sec(S_PREAMP).records(4).zip(&s.input_preamp_db) {
        w.f32(0, *v);
    }
    d.sec(S_MASTER_VOLUME).f32(0, s.master_volume_db);
    {
        let mut w = d.sec(S_INPUT_CONFIG);
        w.u8(ic::INPUT_SOURCE, s.input_source);
        write_pins(
            &mut w,
            ic::SPDIF_RX_PIN,
            ic::SPDIF_RX_PIN_EXT,
            &s.spdif_rx_pins,
        );
        write_pins(&mut w, ic::I2S_RX_PIN, ic::I2S_RX_PIN_EXT, &s.i2s_rx_pins);
        w.u8(ic::I2S_INPUT_RATE, s.i2s_input_rate);
        w.u8(ic::I2S_INPUT_CHANNELS, s.i2s_input_channels);
        // -- WP CG32 -- The four +1-encoded bytes: see `write_p1`.
        let mask = s.spdif_inputs_enabled.wrapping_shr(1) & 0x07;
        write_p1(
            &mut w,
            ic::SPDIF_RX_ENABLED_EXT_P1,
            (s.spdif_inputs_enabled, s.spdif_inputs_enabled_present),
            s.spdif_inputs_enabled != 0x01,
            mask,
            parse_spdif_enabled,
        );
        w.u8(ic::I2S_CLOCK_MODE, s.i2s_clock_mode);
        // 0xFF (unset) goes out as 0, the firmware's mapping
        // (bulk_params.c:252).
        let adat_pin = if s.adat_input_pin == 0xFF {
            0
        } else {
            s.adat_input_pin
        };
        w.u8(ic::ADAT_INPUT_PIN, adat_pin);
        write_p1(
            &mut w,
            ic::ADAT_INPUT_ENABLED_P1,
            (s.adat_input_enabled, s.adat_input_enabled_present),
            s.adat_input_enabled,
            u8::from(s.adat_input_enabled),
            |b| b == 2,
        );
        let mode = s.adat_input_clock_mode;
        write_p1(
            &mut w,
            ic::ADAT_CLOCK_MODE_P1,
            (mode, s.adat_input_clock_mode_present),
            mode != 0,
            mode,
            // An absent byte keeps the state's mode.
            |b| decode_p1(b).map_or(mode, |m| m.min(1)),
        );
    }
    {
        let mut w = d.sec(S_PSYBASS);
        w.flag(psy::ENABLED, s.psybass_enabled);
        w.u16(psy::MASK, s.psybass_output_mask);
        w.f32(psy::CUTOFF, s.psybass_cutoff_hz);
        w.f32(psy::HARMONICS, s.psybass_harmonics_db);
        w.f32(psy::DRIVE, s.psybass_drive_db);
        w.f32(psy::CHARACTER, s.psybass_character_pct);
        w.f32(psy::ORIGINAL, s.psybass_original_db);
    }
    // The firmware ignores the upmixer on RP2040; a bulk SET copies the
    // subharm floats unclamped (the setters clamp).
    encode_params(d.sec(S_UPMIX), &UPMIX_MAP, &s.upmix);
    encode_params(d.sec(S_SUBHARM), &SUBHARM_MAP, &s.subharm);
    encode_params(d.sec(S_TUBE), &TUBE_MAP, &s.tube);
    d.sec(S_LG_SOUND_SYNC).flag(0, s.lg_sound_sync_enabled);
    {
        // Output types and clocks; fields the image marks absent stay absent.
        let mut w = d.sec(S_I2S);
        if let Some(types) = s.output_types.get(..4) {
            w.put(i2s::OUTPUT_TYPES, types);
        }
        w.u8(i2s::BCK_PIN, s.i2s_bck_pin);
        w.u8(i2s::MCK_PIN, s.mck_pin);
        w.flag(i2s::MCK_ENABLED, s.mck_enabled);
        w.u8(i2s::MCK_MULTIPLIER, s.mck_multiplier);
        write_p1(
            &mut w,
            i2s::CLOCK_PIN_MODE_P1,
            (s.i2s_clock_pin_mode, s.i2s_clock_pin_mode_present),
            s.i2s_clock_pin_mode != 0,
            s.i2s_clock_pin_mode,
            |b| b.saturating_sub(1).min(1),
        );
        if w.rd().u8(i2s::BCK_PIN_SLAVE) != 0 {
            w.u8(i2s::BCK_PIN_SLAVE, s.i2s_bck_pin_slave);
        }
    }
    {
        let mut w = d.sec(S_ADAT);
        w.flag(adat::ENABLED, s.adat_out_enabled);
        if w.rd().u8(adat::PIN) != 0 {
            w.u8(adat::PIN, s.adat_out_pin);
        }
    }
    {
        // A bulk SET validates this and saves it to flash, so it must carry
        // the current config, never a stale one.
        let mut w = d.sec(S_DAC_HW_MUTE);
        w.flag(dac::ENABLED, s.dac_mute_enabled);
        w.flag(dac::ACTIVE_LOW, s.dac_mute_active_low);
        w.u8(dac::PIN, s.dac_mute_pin);
        w.u16(dac::HOLD_MS, s.dac_mute_hold_ms);
        w.u16(dac::RELEASE_MS, s.dac_mute_release_ms);
    }
    {
        let mut w = d.sec(S_USER_VOLUME);
        w.f32(0, s.user_volume_db);
        w.flag(4, s.user_mute);
    }
    for (ch, bands) in d.sec(S_CROSSOVERS).records(XOVER_CHANNEL_LEN).zip(&s.xover) {
        for (w, band) in ch.records(BAND_LEN).zip(bands) {
            write_band(w, band);
        }
    }
    let lims = s
        .limiter_enabled
        .iter()
        .zip(&s.limiter_link_group)
        .zip(&s.limiter_threshold_db)
        .zip(&s.limiter_release_ms);
    for (mut w, (((en, group), thr), rel)) in d.sec(S_LIMITER).records(LIMITER_OUTPUT_LEN).zip(lims)
    {
        w.flag(lim::ENABLED, *en);
        w.u8(lim::LINK_GROUP, *group);
        w.f32(lim::THRESHOLD, *thr);
        w.f32(lim::RELEASE, *rel);
    }

    img
}

/// Input-config pins: entry 0 at `first`, entries 1..=3 at `ext..ext + 3`.
fn write_pins(w: &mut Wr<'_>, first: usize, ext: usize, pins: &[u8]) {
    if let Some(p) = pins.first() {
        w.u8(first, *p);
    }
    if let Some(rest) = pins.get(1..4) {
        w.put(ext, rest);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c0 {
    use super::*;
    use proptest::prelude::*;

    const PLATFORM: &[u8] = include_bytes!("../../../tests/fixtures/link/device/platform.bin");
    const BULK: &[u8] = include_bytes!("../../../tests/fixtures/link/device/bulk_v32.bin");
    const EXPECTED_V32: &str = include_str!("../../../tests/fixtures/device/dspstate_v32.json");
    const EXPECTED_DEFAULT: &str =
        include_str!("../../../tests/fixtures/device/dspstate_default.json");

    /// Compare `state` with a fixture field by field (camelCase names),
    /// naming every field that differs.
    fn assert_matches_fixture(state: &DspState, fixture: &str) {
        // Both sides go through JSON text, so f32 values compare as the
        // fixture printed them.
        let ours: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(state).unwrap()).unwrap();
        let want: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let (ours, want) = (ours.as_object().unwrap(), want.as_object().unwrap());
        let cut = |s: String| s.chars().take(200).collect::<String>();
        let mut diff: Vec<String> = want
            .iter()
            .filter(|(k, v)| ours.get(*k) != Some(v))
            .map(|(k, v)| {
                let got = ours.get(k).map_or("<missing>".into(), |g| g.to_string());
                format!("{k}: got {} want {}", cut(got), cut(v.to_string()))
            })
            .collect();
        diff.extend(
            ours.keys()
                .filter(|k| !want.contains_key(*k))
                .map(|k| format!("{k}: not in the fixture")),
        );
        assert!(diff.is_empty(), "fields differ:\n{}", diff.join("\n"));
    }

    fn capture_state() -> DspState {
        let mut s = default_state();
        apply_platform(PLATFORM, &mut s).unwrap();
        decode_bulk(BULK, &mut s).unwrap();
        s.compat = COMPAT_OK;
        s
    }

    /// Everything but `bulk_raw`, NaN-safe (`Debug` prints every NaN alike;
    /// the codec moves float bits unchanged).
    fn fields(s: &DspState) -> String {
        let mut s = s.clone();
        s.bulk_raw.clear();
        format!("{s:?}")
    }

    #[test]
    fn wp_c0_the_capture_decodes_to_the_fixture() {
        assert_matches_fixture(&capture_state(), EXPECTED_V32);
    }

    #[test]
    fn wp_c0_default_state_is_the_fixture() {
        assert_matches_fixture(&default_state(), EXPECTED_DEFAULT);
    }

    #[test]
    fn wp_c0_the_capture_encodes_to_itself() {
        assert_eq!(encode_bulk(&capture_state()), BULK);
    }

    /// The old core's test: notifications patch `encode_bulk(state)` and
    /// decode the result, so a field that is decoded but not encoded would
    /// snap back to the stale value in `bulk_raw`.
    #[test]
    fn wp_c0_every_decoded_field_is_encoded() {
        fn image(seed: u32) -> Vec<u8> {
            // Bytes 0x01..0x3F keep every f32 finite and every name ASCII.
            let mut x = seed;
            let mut d: Vec<u8> = (0..BULK_SIZE)
                .map(|_| {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    1 + ((x >> 24) as u8 % 0x3F)
                })
                .collect();
            d[..8].copy_from_slice(&[V32, 1, 17, 9, 8, 12, 0xF8, 0x17]);
            // Peaking bands: a random Linkwitz Qp would be clamped, not lost.
            for (off, len) in [S_EQ, S_CROSSOVERS] {
                let (bands, _) = d[off..off + len].as_chunks_mut::<BAND_LEN>();
                bands.iter_mut().for_each(|b| b[0] = 1);
            }
            d
        }
        let a = image(1);
        let mut b = image(2);
        b[..g::OFF_GLOBAL].copy_from_slice(&a[..g::OFF_GLOBAL]);
        let mut s1 = default_state();
        decode_bulk(&a, &mut s1).unwrap();
        s1.bulk_raw = b;
        let mut s3 = default_state();
        decode_bulk(&encode_bulk(&s1), &mut s3).unwrap();
        assert_eq!(fields(&s3), fields(&s1));
    }

    #[test]
    fn wp_c0_setters_round_trip_and_unmodeled_bytes_survive() {
        let mut s = capture_state();
        s.bulk_raw[g::OFF_LEGACY + 3] = 0xA5; // legacy section: not modelled
        s.bulk_raw[g::OFF_TUBE + 6] = 0x5A; // tube reserved bytes
        s.preamp_db = -3.5;
        s.input_preamp_db[5] = -2.0;
        s.filters[9][0] = FilterParams {
            filter_type: 11,
            bypass: true,
            freq: 40.0,
            q: 1.2,
            gain: 25.0,
            qp: 0.5,
        };
        s.xover[10][2] = FilterParams {
            filter_type: 35,
            freq: 80.0,
            ..default_band()
        };
        s.matrix_gain[7][8] = -6.0;
        s.limiter_threshold_db[4] = -1.5;
        s.channel_names[0] = "USB L".into();
        s.channel_names[1] = "a name far longer than thirty-one bytes, cut".into();
        s.crossfeed_pair_map = vec![0, 0, 0x54, 0];
        s.upmix[UPMIX_PARAM_PRESENCE as usize] = -3.5;
        s.tube[tube::TUBE_PARAM_TUBE_TYPE as usize] = 7.0;
        s.spdif_inputs_enabled = 0x0B;

        let bytes = encode_bulk(&s);
        assert_eq!(bytes[g::OFF_LEGACY + 3], 0xA5);
        assert_eq!(bytes[g::OFF_TUBE + 6], 0x5A);
        assert_eq!(
            &bytes[g::OFF_CROSSFEED + 12..g::OFF_CROSSFEED + 16],
            &[0, 0, 0x54, 0]
        );
        let mut t = default_state();
        decode_bulk(&bytes, &mut t).unwrap();
        assert_eq!(t.filters[9][0], s.filters[9][0]);
        assert_eq!(t.xover[10][2], s.xover[10][2]);
        assert_eq!(t.channel_names[0], "USB L");
        assert_eq!(t.channel_names[1], "a name far longer than thirty-o");
        s.channel_names[1] = t.channel_names[1].clone();
        // apply_platform's firmware fields and compat are not in the image.
        t.fw_major = s.fw_major;
        t.fw_minor = s.fw_minor;
        t.fw_patch = s.fw_patch;
        t.fw_beta = s.fw_beta;
        t.compat = s.compat;
        assert_eq!(fields(&t), fields(&s));
    }

    #[test]
    fn wp_c0_names_cut_on_a_char_boundary() {
        let mut s = default_state();
        s.bulk_raw[0] = V32;
        s.channel_names[3] = format!("{}é", "x".repeat(30)); // 32 bytes
        let mut t = default_state();
        decode_bulk(&encode_bulk(&s), &mut t).unwrap();
        assert_eq!(t.channel_names[3], "x".repeat(30));
    }

    #[test]
    fn wp_c0_decode_rejects_bad_images_untouched() {
        let mut s = default_state();
        let before = s.clone();
        assert_eq!(
            decode_bulk(&BULK[..100], &mut s),
            Err(WireError::WrongSize {
                got: 100,
                expected: BULK_SIZE
            })
        );
        let mut long = BULK.to_vec();
        long.push(0);
        assert!(matches!(
            decode_bulk(&long, &mut s),
            Err(WireError::WrongSize { .. })
        ));
        let mut v31 = BULK.to_vec();
        v31[0] = 31;
        assert_eq!(
            decode_bulk(&v31, &mut s),
            Err(WireError::UnsupportedVersion {
                got: 31,
                expected: 32
            })
        );
        assert_eq!(s, before);
    }

    #[test]
    fn wp_c0_decode_keeps_absent_fields() {
        let mut img = BULK.to_vec();
        let c = g::OFF_INPUT_CONFIG;
        img[c + ic::I2S_RX_PIN_EXT..c + ic::I2S_RX_PIN_EXT + 3].fill(0);
        img[c + ic::I2S_INPUT_CHANNELS] = 0;
        img[c + ic::ADAT_CLOCK_MODE_P1] = 0;
        img[c + ic::ADAT_INPUT_PIN] = 0;
        img[g::OFF_I2S_CONFIG + i2s::BCK_PIN_SLAVE] = 0;
        img[g::OFF_ADAT_CONFIG + adat::PIN] = 0;
        let mut s = default_state();
        s.i2s_rx_pins = vec![9, 10, 11]; // short: refitted, kept
        s.i2s_input_channels = 6;
        s.adat_input_clock_mode = 1;
        s.i2s_bck_pin_slave = 40;
        s.adat_out_pin = 41;
        decode_bulk(&img, &mut s).unwrap();
        assert_eq!(s.i2s_rx_pins[1..], [10, 11, 4]);
        assert_eq!(s.i2s_input_channels, 6);
        assert_eq!(s.adat_input_clock_mode, 1);
        assert_eq!(s.adat_input_pin, 0xFF);
        assert_eq!((s.i2s_bck_pin_slave, s.adat_out_pin), (40, 41));
        // ... and encoding leaves the absent output pins absent.
        let e = encode_bulk(&s);
        assert_eq!(e[g::OFF_I2S_CONFIG + i2s::BCK_PIN_SLAVE], 0);
        assert_eq!(e[g::OFF_ADAT_CONFIG + adat::PIN], 0);
        assert_eq!(e[c + ic::ADAT_INPUT_PIN], 0);
    }

    #[test]
    fn wp_c0_encode_of_short_vectors_and_raw_does_not_panic() {
        let mut s = default_state();
        s.bulk_raw = vec![V32; 10];
        s.filters = vec![vec![]; 3];
        s.matrix_gain = vec![vec![1.0]];
        s.upmix.clear();
        s.output_pins = vec![1];
        s.crossfeed_pair_map = vec![1, 2];
        s.spdif_rx_pins.clear();
        let e = encode_bulk(&s);
        assert_eq!(e.len(), BULK_SIZE);
        let mut t = default_state();
        decode_bulk(&e, &mut t).unwrap();
        assert_eq!(t.matrix_gain[0][0], 1.0);
    }

    #[test]
    fn wp_c0_apply_platform_full_legacy_and_early_beta() {
        let mut s = default_state();
        apply_platform(&[1, 1, 0x16, 9, 1, 6, 4], &mut s).unwrap();
        assert_eq!((s.platform_id, s.num_output_channels), (1, 9));
        assert_eq!(
            (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
            (1, 1, 6, 4)
        );
        apply_platform(&[0, 1, 0x15, 4, 1, 5], &mut s).unwrap();
        assert_eq!(
            (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
            (1, 1, 5, 0)
        );
        apply_platform(&[0, 1, 0x16, 4, 1, 6], &mut s).unwrap();
        assert_eq!(s.fw_beta, FW_BETA_EARLY);
        apply_platform(&[0, 1, 0x15, 200], &mut s).unwrap();
        assert_eq!((s.fw_minor, s.fw_patch, s.fw_beta), (1, 5, 0));
        assert_eq!(s.num_output_channels, MAX_OUTPUTS as u8);
        let before = s.clone();
        assert_eq!(
            apply_platform(&[1, 2, 3], &mut s),
            Err(WireError::WrongSize {
                got: 3,
                expected: 4
            })
        );
        assert_eq!(s, before);
    }

    #[test]
    fn wp_c0_local_layout_matches_the_generated_tables() {
        let sec = |name: &str| {
            g::SECTIONS
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|&(_, o, l)| (o, l))
                .unwrap()
        };
        assert_eq!(S_I2S, sec("i2s_config"));
        assert_eq!(S_CROSSOVERS, sec("crossovers"));
        assert_eq!(S_ADAT, sec("adat_config"));
        assert_eq!(MATRIX_ROW_LEN * MAX_INPUTS, g::LEN_CROSSPOINTS);
        assert_eq!(OUTPUT_LEN * MAX_OUTPUTS, g::LEN_OUTPUTS);
        assert_eq!(EQ_CHANNEL_LEN * MAX_CHANNELS, g::LEN_EQ);
        assert_eq!(XOVER_CHANNEL_LEN * MAX_CHANNELS, g::LEN_CROSSOVERS);
        assert_eq!(CHANNEL_NAME_LEN * MAX_CHANNELS, g::LEN_CHANNEL_NAMES);
        assert_eq!(LIMITER_OUTPUT_LEN * MAX_OUTPUTS, g::LEN_LIMITER);
        assert_eq!(4 * MAX_CHANNELS, g::LEN_DELAYS);
        assert_eq!(4 * MAX_INPUTS, g::LEN_PREAMP);
        assert_eq!((ic::SPDIF_RX_PIN_EXT, ic::ADAT_CLOCK_MODE_P1), (8, 15));
        assert_eq!(SUBHARM_MAP[9].1, 32);
        assert_eq!(TUBE_MAP[13].1, 40);
    }

    fn v32_image() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(any::<u8>(), BULK_SIZE).prop_map(|mut v| {
            v[0] = V32;
            v
        })
    }

    proptest! {
        /// §1.9: `decode(encode(decode(x)))` equals `decode(x)` for random
        /// V32 images (every field but `bulk_raw`, which is the image).
        #[test]
        fn proptest_wp_c0_decode_encode_decode_is_decode(img in v32_image()) {
            let mut s1 = default_state();
            decode_bulk(&img, &mut s1).unwrap();
            let e1 = encode_bulk(&s1);
            prop_assert_eq!(e1.len(), BULK_SIZE);
            let mut s2 = default_state();
            decode_bulk(&e1, &mut s2).unwrap();
            prop_assert_eq!(fields(&s2), fields(&s1));
            // And encoding is idempotent byte for byte.
            prop_assert_eq!(encode_bulk(&s2), e1);
        }

        /// Arbitrary bytes and platform replies never panic.
        #[test]
        fn proptest_wp_c0_decode_never_panics(
            img in proptest::collection::vec(any::<u8>(), 0..BULK_SIZE + 8),
            reply in proptest::collection::vec(any::<u8>(), 0..10),
        ) {
            let mut s = default_state();
            let _ = decode_bulk(&img, &mut s);
            let _ = apply_platform(&reply, &mut s);
            prop_assert_eq!(encode_bulk(&s).len(), BULK_SIZE);
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg32 {
    use super::*;

    const BULK: &[u8] = include_bytes!("../../../tests/fixtures/link/device/bulk_v32.bin");

    /// Where the four presence bytes sit in the image.
    fn offsets() -> [usize; 4] {
        let c = g::OFF_INPUT_CONFIG;
        [
            g::OFF_I2S_CONFIG + i2s::CLOCK_PIN_MODE_P1,
            c + ic::SPDIF_RX_ENABLED_EXT_P1,
            c + ic::ADAT_INPUT_ENABLED_P1,
            c + ic::ADAT_CLOCK_MODE_P1,
        ]
    }

    fn flags(s: &DspState) -> [bool; 4] {
        [
            s.i2s_clock_pin_mode_present,
            s.spdif_inputs_enabled_present,
            s.adat_input_enabled_present,
            s.adat_input_clock_mode_present,
        ]
    }

    fn decoded(img: &[u8]) -> DspState {
        let mut s = default_state();
        decode_bulk(img, &mut s).unwrap();
        s
    }

    #[test]
    fn wp_cg32_absent_bytes_decode_as_absent_not_as_zero() {
        let mut img = BULK.to_vec();
        for at in offsets() {
            img[at] = 0;
        }
        let s = decoded(&img);
        assert_eq!(flags(&s), [false; 4]);
        // The values beside them read as an absent byte does.
        assert_eq!(s.i2s_clock_pin_mode, 0);
        assert_eq!(s.spdif_inputs_enabled, 0x01);
        assert!(!s.adat_input_enabled);

        // Present and 0 ("+1": the byte is 1) is another thing.
        for at in offsets() {
            img[at] = 1;
        }
        let s = decoded(&img);
        assert_eq!(flags(&s), [true; 4]);
        assert_eq!(
            (
                s.i2s_clock_pin_mode,
                s.spdif_inputs_enabled,
                s.adat_input_enabled,
                s.adat_input_clock_mode
            ),
            (0, 0x01, false, 0)
        );
    }

    #[test]
    fn wp_cg32_presence_round_trips_through_encode() {
        // Every byte value, absent and corrupt ones included, comes back as
        // it was when nothing changed.
        for b in 0..=u8::MAX {
            let mut img = BULK.to_vec();
            for at in offsets() {
                img[at] = b;
            }
            let s = decoded(&img);
            assert_eq!(flags(&s), [b != 0; 4], "byte {b}");
            let e = encode_bulk(&s);
            for at in offsets() {
                assert_eq!(e[at], b, "byte {b} at {at}");
            }
            assert_eq!(flags(&decoded(&e)), flags(&s));
        }
    }

    #[test]
    fn wp_cg32_a_value_set_on_an_absent_byte_becomes_present() {
        let mut img = BULK.to_vec();
        for at in offsets() {
            img[at] = 0;
        }
        let mut s = decoded(&img);
        // What a setter does: the value only.
        s.i2s_clock_pin_mode = 1;
        s.spdif_inputs_enabled = 0x05;
        s.adat_input_enabled = true;
        s.adat_input_clock_mode = 1;
        let t = decoded(&encode_bulk(&s));
        assert_eq!(flags(&t), [true; 4]);
        assert_eq!(
            (
                t.i2s_clock_pin_mode,
                t.spdif_inputs_enabled,
                t.adat_input_enabled,
                t.adat_input_clock_mode
            ),
            (1, 0x05, true, 1)
        );

        // A flag set with the absent reading's value writes it present too,
        // and a flag cleared on a present byte writes it absent.
        let mut s = decoded(&img);
        s.adat_input_enabled_present = true;
        let e = encode_bulk(&s);
        assert_eq!(e[offsets()[2]], 1, "present, disabled");
        let mut s = decoded(&e);
        s.adat_input_enabled_present = false;
        assert_eq!(encode_bulk(&s)[offsets()[2]], 0);
    }

    #[test]
    fn wp_cg32_the_default_state_has_nothing_present() {
        assert_eq!(flags(&default_state()), [false; 4]);
        let mut s = default_state();
        s.bulk_raw[0] = V32;
        assert_eq!(flags(&decoded(&encode_bulk(&s))), [false; 4]);
    }
}
