//! The demo's feature coverage (WP T4b): the panels a feature probe looks
//! for, modelled closely enough that `dspi_session::probe` reports them
//! present and a GUI can drive them.
//!
//! Bulk-backed parameters are rows of [`FEATURE_SCALARS`] (the same model as
//! the parent's `SCALARS`) or of the indexed float tables
//! ([`TUBE_PARAMS`], [`LIMITER_PARAMS`], [`UPMIX_PARAMS`]), with the struct
//! layouts below (`bulk_params.h`) checked against the generated section
//! lengths. The rest are small non-bulk models: the upmixer status, input
//! source and board names (with their `0x0F`/`0x10` notifications), the
//! output latency option, and the UART/I2C control-interface configs.
//!
//! Firmware behaviour that is not modelled: the platform split (the upmixer
//! and ADAT answer as on an RP2350 whatever the capture), pin validation
//! (any GPIO is accepted, nothing is "in use"), the tube type's row presets
//! (a type change does not rewrite bias/asym/hardness/sag), limiter group
//! ganging on `LIMITER_ALL_OUTPUTS`, and the deferred applies (an input
//! source, clock mode or DAC mute config takes effect, and notifies, at
//! once; the output latency does too unless a test asks otherwise, see
//! [`FakeRemote::defer_latency`]). Nor are the float range clamps of the psybass, subharm, leveller,
//! tube and limiter parameters (a float is stored as sent; only the byte
//! and integer fields are clamped), the tube's `custom_reset_if` (moving
//! bias/asym/hardness/sag does not reset the type to custom), ADAT input
//! selectability needing `adat_input_pin != 0xFF` as well as the enable,
//! and S/PDIF enable acceptance depending on the input's RX pin (any enable
//! is accepted).
//!
//! Rounding: integer fields set from a float use [`f32::round`] (ties away
//! from zero), where the firmware's `round_clamp_u` adds 0.5 and truncates
//! and `lrintf` rounds ties to even; they differ only at exact .5 values.
//! The upmixer's modes and presence follow `lrintf` (ties to even).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::{limiter as lim, notify as nfy, opcodes as op, subharm, tube};
use dspi_proto::packets::{I2cCtrlConfig, UartCtrlConfig, UpmixStatus};
use dspi_proto::wire::{
    INPUT_CONFIG_FIELDS, LIMITER_OUTPUT_FIELDS, LIMITER_OUTPUT_LEN, SUBHARM_FIELDS, TUBE_FIELDS,
};

use super::{FakeRemote, Field, Index, Scalar, State, Store, scalar};

// ------------------------------------------------------------------ layouts

// `WireLevellerConfig` (bulk_params.h:175-186).
pub(super) const L_ENABLED: Field = ("enabled", 0, 1);
pub(super) const L_SPEED: Field = ("speed", 1, 1);
pub(super) const L_LOOKAHEAD: Field = ("lookahead", 2, 1);
// -- WP CG16 --
impl State {
    /// `i2c_ctrl_live_instance`: which I2C block the control interface holds. A
    /// display may not take it.
    pub(super) fn i2c_ctrl_live_instance(&self) -> Option<u8> {
        let cfg = I2cCtrlConfig::decode(&self.extras.i2c).ok()?;
        cfg.enabled.then_some((cfg.sda_pin >> 1) & 1)
    }
}

#[cfg(test)]
pub(super) const L_RESERVED: Field = ("reserved", 3, 1);
pub(super) const L_AMOUNT: Field = ("amount", 4, 4);
pub(super) const L_MAX_GAIN: Field = ("max_gain_db", 8, 4);
pub(super) const L_GATE: Field = ("gate_threshold_db", 12, 4);
pub(super) const L_DETECTOR: Field = ("detector_mask", 16, 1);
pub(super) const L_APPLY: Field = ("apply_mask", 17, 1);
#[cfg(test)]
pub(super) const L_RESERVED2: Field = ("reserved2", 18, 2);

// `WirePsybassParams` (bulk_params.h:344-353).
pub(super) const P_ENABLED: Field = ("enabled", 0, 1);
#[cfg(test)]
pub(super) const P_RESERVED: Field = ("reserved0", 1, 1);
pub(super) const P_MASK: Field = ("output_mask", 2, 2);
pub(super) const P_CUTOFF: Field = ("cutoff_hz", 4, 4);
pub(super) const P_HARMONICS: Field = ("harmonics_db", 8, 4);
pub(super) const P_DRIVE: Field = ("drive_db", 12, 4);
pub(super) const P_CHARACTER: Field = ("character_pct", 16, 4);
pub(super) const P_ORIGINAL: Field = ("original_db", 20, 4);

// `WireUpmixParams` (bulk_params.h:363-378), also `UpmixConfigPacket`
// (upmix.h:167-182), byte for byte.
pub(super) const U_ENABLED: Field = ("enabled", 0, 1);
pub(super) const U_CENTER: Field = ("center_mode", 1, 1);
pub(super) const U_SURROUND: Field = ("surround_mode", 2, 1);
pub(super) const U_PRESENCE: Field = ("presence_q1", 3, 1);
pub(super) const U_STRENGTH: Field = ("strength_pct", 4, 4);
pub(super) const U_WIDTH: Field = ("center_width_pct", 8, 4);
pub(super) const U_THRESHOLD: Field = ("corr_threshold_pct", 12, 4);
pub(super) const U_ATTACK: Field = ("attack_ms", 16, 4);
pub(super) const U_RELEASE: Field = ("release_ms", 20, 4);
pub(super) const U_DET_HPF: Field = ("detector_hpf_hz", 24, 4);
pub(super) const U_SUR_DELAY: Field = ("surround_delay_ms", 28, 4);
pub(super) const U_SUR_HPF: Field = ("surround_hpf_hz", 32, 4);
pub(super) const U_SUR_LPF: Field = ("surround_lpf_hz", 36, 4);
pub(super) const U_DECORR: Field = ("decorr_pct", 40, 4);
const UPMIX_LEN: usize = 44;

// `WireI2SConfig` (bulk_params.h:157-170).
#[cfg(test)]
pub(super) const I_OUTPUT_TYPES: Field = ("output_types", 0, 4);
pub(super) const I_BCK: Field = ("bck_pin", 4, 1);
pub(super) const I_MCK_PIN: Field = ("mck_pin", 5, 1);
pub(super) const I_MCK_ENABLED: Field = ("mck_enabled", 6, 1);
pub(super) const I_MCK_MULT: Field = ("mck_multiplier", 7, 1);
#[cfg(test)]
pub(super) const I_CLOCK_PIN_MODE: Field = ("clock_pin_mode_p1", 8, 1);
pub(super) const I_BCK_SLAVE: Field = ("bck_pin_slave", 9, 1);
#[cfg(test)]
pub(super) const I_RESERVED: Field = ("reserved", 10, 6);

// `WireLgSoundSync` (bulk_params.h:252-258): `enabled` is the only field a
// host sets; the rest is the detector's state, read-only.
pub(super) const LG_ENABLED: Field = ("enabled", 0, 1);
#[cfg(test)]
pub(super) const LG_REST: [Field; 4] = [
    ("present", 1, 1),
    ("volume", 2, 1),
    ("muted", 3, 1),
    ("reserved", 4, 12),
];
const LG_LEN: usize = 16;

// `WireAdatConfig` (bulk_params.h:331-335).
pub(super) const A_ENABLED: Field = ("enabled", 0, 1);
pub(super) const A_PIN: Field = ("pin", 1, 1);
#[cfg(test)]
pub(super) const A_RESERVED: Field = ("reserved", 2, 6);

/// `WireDacHwMute` (bulk_params.h:288-296), the same 16 bytes as the
/// `DacHwMuteConfig` the GET/SET carry.
const DAC_HW_MUTE_LEN: usize = 16;

// `WireInputConfig` fields, from `dspi_proto::wire::INPUT_CONFIG_FIELDS`
// (the tests check each name).
pub(super) const IC_SOURCE: Field = INPUT_CONFIG_FIELDS[0];
pub(super) const IC_SPDIF_PIN: Field = INPUT_CONFIG_FIELDS[1];
pub(super) const IC_I2S_RX_PIN: Field = INPUT_CONFIG_FIELDS[2];
pub(super) const IC_I2S_RATE: Field = INPUT_CONFIG_FIELDS[3];
pub(super) const IC_I2S_CHANNELS: Field = INPUT_CONFIG_FIELDS[4];
pub(super) const IC_I2S_RX_PIN_EXT: Field = INPUT_CONFIG_FIELDS[5];
pub(super) const IC_SPDIF_PIN_EXT: Field = INPUT_CONFIG_FIELDS[6];
pub(super) const IC_SPDIF_ENABLED_P1: Field = INPUT_CONFIG_FIELDS[7];
pub(super) const IC_I2S_CLOCK_MODE: Field = INPUT_CONFIG_FIELDS[8];
pub(super) const IC_ADAT_PIN: Field = INPUT_CONFIG_FIELDS[9];
pub(super) const IC_ADAT_ENABLED_P1: Field = INPUT_CONFIG_FIELDS[10];
pub(super) const IC_ADAT_CLOCK_P1: Field = INPUT_CONFIG_FIELDS[11];

// `WireSubharmParams`, from `dspi_proto::wire::SUBHARM_FIELDS`.
pub(super) const SH_ENABLED: Field = SUBHARM_FIELDS[0];
pub(super) const SH_MASK: Field = SUBHARM_FIELDS[2];
pub(super) const SH_LOW: Field = SUBHARM_FIELDS[3];
pub(super) const SH_HIGH: Field = SUBHARM_FIELDS[4];
pub(super) const SH_BOOST: Field = SUBHARM_FIELDS[5];
pub(super) const SH_TOP: Field = SUBHARM_FIELDS[6];
pub(super) const SH_DEPTH: Field = SUBHARM_FIELDS[7];
pub(super) const SH_HOLD: Field = SUBHARM_FIELDS[8];
pub(super) const SH_CEILING: Field = SUBHARM_FIELDS[9];
pub(super) const SH_SELECT: Field = SUBHARM_FIELDS[10];
pub(super) const SH_LINK: Field = SUBHARM_FIELDS[11];

/// `LEVELLER_SPEED_COUNT` (leveller.h:58), not generated: a larger speed is
/// ignored.
const LEVELLER_SPEED_MAX: u8 = 2;

/// `SPDIF_RX_NUM_INPUTS` (audio_input.h:31), both platforms.
const SPDIF_INPUTS: u8 = 4;
/// `INPUT_SOURCE_*` (audio_input.h:16-26): USB, S/PDIF 1, I2S, ADAT, then
/// S/PDIF 2-4.
const SRC_ADAT: u8 = 3;
const SRC_SPDIF2: u8 = 4;
const SRC_MAX: u8 = 6;
/// `I2S_RX_MAX_PAIRS` (audio_input.h:65-67): 4 on an RP2350, 1 on an RP2040.
fn i2s_pairs(platform_id: u8) -> u8 {
    if platform_id == 0 { 1 } else { 4 }
}

/// `PIN_CONFIG_*` (config.h:607-613).
mod pin {
    use dspi_proto::generated::status as st;
    pub const OK: u8 = st::PIN_CONFIG_SUCCESS as u8;
    pub const INVALID_PIN: u8 = st::PIN_CONFIG_INVALID_PIN as u8;
    pub const PIN_IN_USE: u8 = st::PIN_CONFIG_PIN_IN_USE as u8;
    pub const INVALID_OUTPUT: u8 = st::PIN_CONFIG_INVALID_OUTPUT as u8;
    pub const INVALID_PARAM: u8 = st::PIN_CONFIG_INVALID_PARAM as u8;
}

// ------------------------------------------------------------- the scalars

/// Plain GET/SET pairs onto one bulk field, as the parent's `SCALARS`.
pub(super) const FEATURE_SCALARS: &[Scalar] = &[
    // Leveller.
    scalar(
        op::REQ_SET_LEVELLER_ENABLE,
        op::REQ_GET_LEVELLER_ENABLE,
        "leveller",
        L_ENABLED,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_LEVELLER_AMOUNT,
        op::REQ_GET_LEVELLER_AMOUNT,
        "leveller",
        L_AMOUNT,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_LEVELLER_SPEED,
        op::REQ_GET_LEVELLER_SPEED,
        "leveller",
        L_SPEED,
        Store::Max(LEVELLER_SPEED_MAX),
    ),
    scalar(
        op::REQ_SET_LEVELLER_MAX_GAIN,
        op::REQ_GET_LEVELLER_MAX_GAIN,
        "leveller",
        L_MAX_GAIN,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_LEVELLER_LOOKAHEAD,
        op::REQ_GET_LEVELLER_LOOKAHEAD,
        "leveller",
        L_LOOKAHEAD,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_LEVELLER_GATE,
        op::REQ_GET_LEVELLER_GATE,
        "leveller",
        L_GATE,
        Store::Raw,
    ),
    // Psychoacoustic bass.
    scalar(
        op::REQ_SET_PSYBASS,
        op::REQ_GET_PSYBASS,
        "psybass",
        P_ENABLED,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_PSYBASS_MASK,
        op::REQ_GET_PSYBASS_MASK,
        "psybass",
        P_MASK,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_PSYBASS_CUTOFF,
        op::REQ_GET_PSYBASS_CUTOFF,
        "psybass",
        P_CUTOFF,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_PSYBASS_HARMONICS,
        op::REQ_GET_PSYBASS_HARMONICS,
        "psybass",
        P_HARMONICS,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_PSYBASS_DRIVE,
        op::REQ_GET_PSYBASS_DRIVE,
        "psybass",
        P_DRIVE,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_PSYBASS_CHARACTER,
        op::REQ_GET_PSYBASS_CHARACTER,
        "psybass",
        P_CHARACTER,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_PSYBASS_ORIGINAL,
        op::REQ_GET_PSYBASS_ORIGINAL,
        "psybass",
        P_ORIGINAL,
        Store::Raw,
    ),
    // Subharmonic synthesizer (solo is runtime-only, not on the wire).
    scalar(
        op::REQ_SET_SUBHARM,
        op::REQ_GET_SUBHARM,
        "subharm",
        SH_ENABLED,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_SUBHARM_MASK,
        op::REQ_GET_SUBHARM_MASK,
        "subharm",
        SH_MASK,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_LOW,
        op::REQ_GET_SUBHARM_LOW,
        "subharm",
        SH_LOW,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_HIGH,
        op::REQ_GET_SUBHARM_HIGH,
        "subharm",
        SH_HIGH,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_BOOST,
        op::REQ_GET_SUBHARM_BOOST,
        "subharm",
        SH_BOOST,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_TOP,
        op::REQ_GET_SUBHARM_TOP,
        "subharm",
        SH_TOP,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_DEPTH,
        op::REQ_GET_SUBHARM_DEPTH,
        "subharm",
        SH_DEPTH,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_HOLD,
        op::REQ_GET_SUBHARM_HOLD,
        "subharm",
        SH_HOLD,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_CEILING,
        op::REQ_GET_SUBHARM_CEILING,
        "subharm",
        SH_CEILING,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_SUBHARM_SELECT,
        op::REQ_GET_SUBHARM_SELECT,
        "subharm",
        SH_SELECT,
        Store::Clamp(subharm::SUBHARM_SELECT_MODE_MAX as u8),
    ),
    scalar(
        op::REQ_SET_SUBHARM_LINK,
        op::REQ_GET_SUBHARM_LINK,
        "subharm",
        SH_LINK,
        Store::Bool,
    ),
    // LG Sound Sync's user gate.
    scalar(
        op::REQ_SET_LG_SOUND_SYNC_ENABLE,
        op::REQ_GET_LG_SOUND_SYNC_ENABLE,
        "lg_sound_sync",
        LG_ENABLED,
        Store::Bool,
    ),
    // I2S clock mode: 0 master, 1 slave; anything else is ignored.
    scalar(
        op::REQ_SET_I2S_CLOCK_MODE,
        op::REQ_GET_I2S_CLOCK_MODE,
        "input_config",
        IC_I2S_CLOCK_MODE,
        Store::Max(1),
    ),
];

/// GET-only rows: a field read back as is (the matching SET is a
/// write-as-read with a status byte, modelled in `State::get_feature`).
struct Getter {
    get: u8,
    section: &'static str,
    field: Field,
    /// Record stride and index space when indexed by `wValue`'s low byte.
    stride: usize,
    index: Index,
}

const fn getter(get: u8, section: &'static str, field: Field) -> Getter {
    Getter {
        get,
        section,
        field,
        stride: 0,
        index: Index::None,
    }
}

const GETTERS: &[Getter] = &[
    getter(op::REQ_GET_ADAT_ENABLE, "adat_config", A_ENABLED),
    getter(op::REQ_GET_ADAT_PIN, "adat_config", A_PIN),
    getter(op::REQ_GET_MCK_ENABLE, "i2s_config", I_MCK_ENABLED),
    getter(op::REQ_GET_MCK_PIN, "i2s_config", I_MCK_PIN),
    getter(op::REQ_GET_MCK_MULTIPLIER, "i2s_config", I_MCK_MULT),
    Getter {
        get: op::REQ_GET_OUTPUT_TYPE,
        section: "i2s_config",
        field: ("output_type", 0, 1),
        stride: 1,
        index: Index::Spdif,
    },
];

// ---------------------------------------------- indexed float parameters

/// How a float on the wire maps onto a bulk field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Num {
    /// `!= 0` as 0/1.
    Bool,
    /// Rounded and clamped to `0..=max`.
    U8(u8),
    /// Rounded and clamped to `0..=65535`.
    U16,
    /// As is.
    F32,
    /// `int8`, half-dB steps (`presence_q1`): `v` clamped to
    /// `UPMIX_PRESENCE_MIN..=MAX` (+-12 dB, upmix.h:108-109), then
    /// `lrintf(v * 2)`, so -24..=24 (`upmix_presence_encode`, upmix.h:206-209).
    Half,
    /// A mode byte: `lrintf(v)`, and anything outside `0..=max` becomes
    /// `fallback` (`upmix_clamp_center_mode`, upmix.h:129-131).
    Choice { max: u8, fallback: u8 },
    /// A mode byte: `lrintf(v)` clamped to `0..=max` (the surround mode,
    /// `REQ_UPMIX_SET_PARAM`).
    ModeClamp(u8),
}

impl Num {
    fn encode(self, v: f32) -> Vec<u8> {
        // Float-to-int `as` saturates; the clamps keep it in range anyway.
        match self {
            Num::Bool => vec![u8::from(v != 0.0)],
            Num::U8(max) => vec![v.round().clamp(0.0, f32::from(max)) as u8],
            Num::U16 => (v.round().clamp(0.0, 65535.0) as u16)
                .to_le_bytes()
                .to_vec(),
            Num::F32 => v.to_le_bytes().to_vec(),
            Num::Half => vec![(v.clamp(-12.0, 12.0) * 2.0).round_ties_even() as i8 as u8],
            Num::ModeClamp(max) => vec![v.round_ties_even().clamp(0.0, f32::from(max)) as u8],
            Num::Choice { max, fallback } => {
                let m = v.round_ties_even();
                vec![if (0.0..=f32::from(max)).contains(&m) {
                    m as u8
                } else {
                    fallback
                }]
            }
        }
    }

    fn decode(self, b: &[u8]) -> Option<f32> {
        Some(match self {
            Num::Bool | Num::U8(_) | Num::ModeClamp(_) | Num::Choice { .. } => {
                f32::from(*b.first()?)
            }
            Num::U16 => f32::from(u16::from_le_bytes(b.get(..2)?.try_into().ok()?)),
            Num::F32 => f32::from_le_bytes(b.get(..4)?.try_into().ok()?),
            Num::Half => f32::from(i8::from_le_bytes([*b.first()?])) / 2.0,
        })
    }
}

/// Indexed by `TUBE_PARAM_*` (tube.h:16-31); `tube_set_param`/`_get_param`.
pub(super) const TUBE_PARAMS: [(Field, Num); 14] = [
    (TUBE_FIELDS[0], Num::Bool),
    (TUBE_FIELDS[4], Num::U16),
    (TUBE_FIELDS[1], Num::U8(tube::TUBE_TYPE_MAX as u8)),
    (TUBE_FIELDS[6], Num::F32),
    (TUBE_FIELDS[7], Num::F32),
    (TUBE_FIELDS[8], Num::F32),
    (TUBE_FIELDS[9], Num::F32),
    (TUBE_FIELDS[10], Num::F32),
    (TUBE_FIELDS[2], Num::U8(tube::TUBE_RECT_MAX as u8)),
    (TUBE_FIELDS[3], Num::Bool),
    (TUBE_FIELDS[11], Num::F32),
    (TUBE_FIELDS[12], Num::F32),
    (TUBE_FIELDS[13], Num::F32),
    (TUBE_FIELDS[14], Num::F32),
];

/// Indexed by `LIMITER_PARAM_*` (limiter.h:12-17), one record per output.
pub(super) const LIMITER_PARAMS: [(Field, Num); 4] = [
    (LIMITER_OUTPUT_FIELDS[0], Num::Bool),
    (LIMITER_OUTPUT_FIELDS[3], Num::F32),
    (LIMITER_OUTPUT_FIELDS[4], Num::F32),
    (
        LIMITER_OUTPUT_FIELDS[1],
        Num::U8(lim::LIMITER_LINK_GROUP_MAX as u8),
    ),
];

/// `UPMIX_DEFAULT_CENTER_MODE` (upmix.h), adaptive; not generated.
const UPMIX_DEFAULT_CENTER: u8 = 1;

/// Indexed by `UPMIX_PARAM_*` (upmix.h:187-202). `GET_PARAM 13` here reads
/// the stored half-dB byte back; the firmware returns its unclamped float
/// `presence_db`, so a value set outside +-12 dB reads back differently.
pub(super) const UPMIX_PARAMS: [(Field, Num); 14] = [
    (U_ENABLED, Num::Bool),
    (
        U_CENTER,
        Num::Choice {
            max: 2,
            fallback: UPMIX_DEFAULT_CENTER,
        },
    ),
    (U_SURROUND, Num::ModeClamp(2)),
    (U_STRENGTH, Num::F32),
    (U_WIDTH, Num::F32),
    (U_THRESHOLD, Num::F32),
    (U_ATTACK, Num::F32),
    (U_RELEASE, Num::F32),
    (U_DET_HPF, Num::F32),
    (U_SUR_DELAY, Num::F32),
    (U_SUR_HPF, Num::F32),
    (U_SUR_LPF, Num::F32),
    (U_DECORR, Num::F32),
    (U_PRESENCE, Num::Half),
];

// ------------------------------------------------------- non-bulk state

/// `INPUT_SOURCE_NAME_LEN` / `DEVICE_NAME_LEN`.
const NAME_LEN: usize = nfy::NOTIFY_INPUT_SOURCE_NAME_LEN as usize;
/// `OUTPUT_LATENCY_DEFAULT`, the 4 ms option (output_latency.h).
const LATENCY_DEFAULT: u8 = 5;
/// `output_latency_options[]` (output_latency.c:20-28): id, flags,
/// latency_us, batch_us. Flag 1 is `OUTPUT_LATENCY_FLAG_STREAM_ONLY`.
const LATENCY_OPTIONS: [(u8, u8, u16, u16); 7] = [
    (1, 1, 667, 333),
    (2, 1, 1000, 333),
    (3, 0, 2000, 1000),
    (4, 0, 3000, 1000),
    (5, 0, 4000, 1000),
    (6, 0, 6000, 1000),
    (7, 0, 8000, 1000),
];
/// `OUTPUT_LATENCY_WIRE_AVAILABLE` (config.h:1214).
const LATENCY_AVAILABLE: u8 = 0x80;
const RATE_HZ: u64 = 48_000;

/// What the device keeps outside the bulk image.
pub(super) struct Extras {
    /// Per input source, `0..=SRC_MAX`.
    source_names: Vec<[u8; NAME_LEN]>,
    device_name: [u8; NAME_LEN],
    latency: u8,
    uart: [u8; UartCtrlConfig::SIZE],
    i2c: [u8; I2cCtrlConfig::SIZE],
    // -- WP TM20 --
    latency_apply: LatencyApply,
}

// -- WP TM20 --
/// How a latency SET lands, set through [`FakeRemote`]. By default at once.
#[derive(Default)]
struct LatencyApply {
    /// Status reads a SET stays pending for; `None` applies at once.
    reads: Option<u32>,
    /// A pending SET never finishes.
    hold: bool,
    /// A SET is ACKed and dropped: nothing changes, nothing stays pending.
    refuse: bool,
    /// Status reads that stall while a SET is pending, as the flash window does.
    blackout: u32,
    /// The id waiting for the main loop, and the status reads left.
    pending: Option<(u8, u32)>,
    /// Every SET of a known id that arrived.
    sets: u32,
}

/// Simulated firmware timing for the output latency (`vendor_commands.c:1474`,
/// `main.c:2298`): the SET is noted, the main loop applies it later, and the
/// status reports the old id with `pending` set until then.
impl FakeRemote {
    /// A SET stays pending for this many status reads, then applies.
    pub fn defer_latency(&self, reads: u32) {
        self.shared.lock().extras.latency_apply.reads = Some(reads);
    }

    /// A SET stays pending for good.
    pub fn hold_latency(&self) {
        let mut s = self.shared.lock();
        s.extras.latency_apply.reads = Some(0);
        s.extras.latency_apply.hold = true;
    }

    /// A SET is ACKed and dropped. With [`Self::defer_latency`], it is dropped
    /// when the apply comes round.
    pub fn refuse_latency(&self) {
        self.shared.lock().extras.latency_apply.refuse = true;
    }

    /// While a SET is pending, this many status reads stall first.
    pub fn black_out_latency(&self, reads: u32) {
        self.shared.lock().extras.latency_apply.blackout = reads;
    }

    /// How many latency SETs the device has received.
    pub fn latency_sets(&self) -> u32 {
        self.shared.lock().extras.latency_apply.sets
    }
}

impl State {
    // -- WP QD5 --
    /// [`super::EXT_FAKE_LATENCY_TIMING`]'s OUT path. False refuses it.
    pub(super) fn latency_timing(&mut self, data: &[u8]) -> bool {
        let &[mode, reads, blackout] = data else {
            return false;
        };
        let (reads, hold, refuse) = match mode {
            0 => (None, false, false),
            1 => (Some(u32::from(reads)), false, false),
            2 => (Some(0), true, false),
            3 => (Some(u32::from(reads)), false, true),
            _ => return false,
        };
        let a = &mut self.extras.latency_apply;
        a.reads = reads;
        a.hold = hold;
        a.refuse = refuse;
        a.blackout = u32::from(blackout);
        true
    }

    // -- WP QD5 --
    /// How many latency SETs arrived.
    pub(super) fn latency_sets(&self) -> u32 {
        self.extras.latency_apply.sets
    }

    /// `REQ_OUTPUT_LATENCY`'s OUT path for a known id.
    fn latency_set(&mut self, id: u8) {
        let a = &mut self.extras.latency_apply;
        a.sets = a.sets.saturating_add(1);
        match a.reads {
            Some(reads) => a.pending = Some((id, reads)),
            None if a.refuse => {}
            None => self.extras.latency = id,
        }
    }

    /// One status read's effect on a pending SET; `None` is a stall.
    fn latency_status_read(&mut self) -> Option<Vec<u8>> {
        let a = &mut self.extras.latency_apply;
        if a.pending.is_some() && a.blackout > 0 {
            a.blackout = a.blackout.saturating_sub(1);
            return None;
        }
        let status = self.latency_status();
        let a = &mut self.extras.latency_apply;
        if let Some((id, left)) = a.pending {
            if a.hold {
                return Some(status);
            }
            if left > 0 {
                a.pending = Some((id, left.saturating_sub(1)));
                return Some(status);
            }
            a.pending = None;
            if !a.refuse {
                self.extras.latency = id;
            }
            return Some(self.latency_status());
        }
        Some(status)
    }
}

impl Extras {
    pub(super) fn new() -> Self {
        let mut device_name = [0; NAME_LEN];
        put_name(&mut device_name, b"DSPi Demo");
        Self {
            source_names: vec![[0; NAME_LEN]; usize::from(SRC_MAX).saturating_add(1)],
            device_name,
            latency: LATENCY_DEFAULT,
            uart: UartCtrlConfig::default().encode(),
            i2c: I2cCtrlConfig::default().encode(),
            // -- WP TM20 --
            latency_apply: LatencyApply::default(),
        }
    }
}

// -- WP P1OF --
impl Extras {
    /// The device's name, as `REQ_SET_DEVICE_NAME` would store it (a document
    /// opened offline names the device).
    pub(super) fn set_device_name(&mut self, name: &[u8]) {
        put_name(&mut self.device_name, name);
    }
}

/// `stage_entry` (input_source_names.c:29-41): at most 31 bytes, a leading
/// NUL clears, NUL padded.
fn put_name(dst: &mut [u8; NAME_LEN], src: &[u8]) {
    *dst = [0; NAME_LEN];
    if src.first().is_some_and(|b| *b != 0) {
        let n = src.len().min(NAME_LEN.saturating_sub(1));
        if let (Some(d), Some(s)) = (dst.get_mut(..n), src.get(..n)) {
            d.copy_from_slice(s);
        }
    }
}

// ------------------------------------------------------------ dispatch

impl State {
    fn field_bytes(&self, section: &str, stride: usize, i: usize, f: Field) -> Option<Vec<u8>> {
        self.bytes(self.locate(section, stride, i, f)?, f.2)
    }

    fn put(&mut self, section: &str, stride: usize, i: usize, f: Field, bytes: &[u8]) -> bool {
        let host = nfy::PARAM_SRC_HOST_SET as u8;
        self.locate(section, stride, i, f)
            .is_some_and(|off| self.write(off, bytes, host))
    }

    // -- WP CS1 --
    /// The subharmonic config the headroom bound reads; `None` when the
    /// image has no subharmonic section.
    fn subharm_config(&self) -> Option<super::subharm::Config> {
        let f32_of = |f: Field| {
            let b = self.field_bytes("subharm", 0, 0, f)?;
            Some(f32::from_le_bytes(b.get(..4)?.try_into().ok()?))
        };
        Some(super::subharm::Config {
            enabled: self.field_bytes("subharm", 0, 0, SH_ENABLED)?.first() != Some(&0),
            low_db: f32_of(SH_LOW)?,
            high_db: f32_of(SH_HIGH)?,
            top_db: f32_of(SH_TOP)?,
            boost_db: f32_of(SH_BOOST)?,
            ceiling_db: f32_of(SH_CEILING)?,
        })
    }

    fn ic(&self, f: Field) -> Option<u8> {
        self.field_bytes("input_config", 0, 0, f)?.first().copied()
    }

    /// A `_p1` field: `None` when absent (0), else the value.
    fn ic_p1(&self, f: Field) -> Option<u8> {
        self.ic(f)?.checked_sub(1)
    }

    fn spdif_enabled_ext(&self) -> u8 {
        self.ic_p1(IC_SPDIF_ENABLED_P1).unwrap_or(0)
    }

    fn adat_input_enabled(&self) -> u8 {
        self.ic_p1(IC_ADAT_ENABLED_P1).unwrap_or(0)
    }

    /// `input_source_selectable` (audio_input.h): the always-on inputs, ADAT
    /// once enabled, and an optional S/PDIF input once enabled.
    fn selectable(&self, src: u8) -> bool {
        match src {
            0..SRC_ADAT => true,
            SRC_ADAT => self.adat_input_enabled() != 0,
            SRC_SPDIF2..=SRC_MAX => {
                let bit = src.saturating_sub(SRC_SPDIF2);
                self.spdif_enabled_ext()
                    .checked_shr(u32::from(bit))
                    .unwrap_or(0)
                    & 1
                    != 0
            }
            // -- WP SW1 -- the audio link input
            dspi_proto::ext::alink::INPUT_SOURCE_ALINK => self.alink_input(),
            _ => false,
        }
    }

    /// Feature GETs, and the write-as-read SETs among them. `None`: not a
    /// feature opcode; `Some(None)`: stall.
    pub(super) fn get_feature(&mut self, opcode: u8, value: u16) -> Option<Option<Vec<u8>>> {
        let [hi, lo] = value.to_be_bytes();
        if let Some(g) = GETTERS.iter().find(|g| g.get == opcode) {
            let i = if g.index == Index::None {
                0
            } else {
                usize::from(lo)
            };
            if i >= self.bound(g.index) {
                return Some(None);
            }
            return Some(self.field_bytes(g.section, g.stride, i, g.field));
        }
        let r = match opcode {
            op::REQ_GET_INPUT_SOURCE => self.ic(IC_SOURCE).map(|v| vec![v]),
            // -- WP CS1 -- computed from the live config, as the firmware does.
            op::REQ_GET_SUBHARM_HEADROOM => self
                .subharm_config()
                .map(|c| super::subharm::headroom_db(c).to_le_bytes().to_vec()),
            op::REQ_GET_LEVELLER_MASKS => {
                let d = self.field_bytes("leveller", 0, 0, L_DETECTOR);
                let a = self.field_bytes("leveller", 0, 0, L_APPLY);
                d.zip(a).map(|(d, a)| [d, a].concat())
            }
            op::REQ_GET_I2S_INPUT_CHANNELS => {
                // The live count is never 0; an absent field reads stereo.
                let c = self.ic(IC_I2S_CHANNELS).unwrap_or(0);
                Some(vec![if c == 0 { 2 } else { c }])
            }
            op::REQ_GET_I2S_RX_PIN => Some(vec![self.i2s_rx_pin(lo).unwrap_or(0)]),
            op::REQ_GET_SPDIF_RX_PIN => Some(vec![self.spdif_pin(lo).unwrap_or(0)]),
            op::REQ_GET_SPDIF_INPUT_CONFIG => {
                let mut r = vec![
                    SPDIF_INPUTS,
                    self.spdif_enabled_ext()
                        .checked_shl(1)
                        .unwrap_or(0)
                        .wrapping_add(1),
                ];
                r.extend((0..SPDIF_INPUTS).map(|i| self.spdif_pin(i).unwrap_or(0)));
                Some(r)
            }
            op::REQ_GET_INPUT_RATE => {
                let hz: u32 = match self.ic(IC_I2S_RATE) {
                    Some(0) => 44_100,
                    Some(2) => 96_000,
                    _ => 48_000,
                };
                let mut r = 48_000u32.to_le_bytes().to_vec();
                r.extend(hz.to_le_bytes());
                Some(r)
            }
            op::REQ_GET_ADAT_INPUT_ENABLE => Some(vec![self.adat_input_enabled()]),
            op::REQ_GET_ADAT_INPUT_PIN => Some(vec![match self.ic(IC_ADAT_PIN) {
                Some(0) | None => 0xFF,
                Some(p) => p,
            }]),
            op::REQ_GET_ADAT_INPUT_CLOCK_MODE => {
                Some(vec![self.ic_p1(IC_ADAT_CLOCK_P1).unwrap_or(0)])
            }
            // Role 0 the master/unified pair, 1 the slave pair, else 0.
            op::REQ_GET_I2S_BCK_PIN => match lo {
                0 => self.field_bytes("i2s_config", 0, 0, I_BCK),
                1 => self.field_bytes("i2s_config", 0, 0, I_BCK_SLAVE),
                _ => Some(vec![0]),
            },
            op::REQ_GET_LG_SOUND_SYNC_STATUS => {
                self.field_bytes("lg_sound_sync", 0, 0, ("status", 0, LG_LEN))
            }
            op::REQ_GET_DAC_HW_MUTE_CONFIG => {
                self.field_bytes("dac_hw_mute", 0, 0, ("config", 0, DAC_HW_MUTE_LEN))
            }
            op::REQ_GET_TUBE_PARAM => self.get_param("tube", 0, 0, &TUBE_PARAMS, lo),
            op::REQ_LIMITER => self.get_limiter(hi, lo),
            op::REQ_UPMIX_GET_PARAM => {
                if value >= UPMIX_PARAMS.len() as u16 {
                    None
                } else {
                    self.get_param("upmix", 0, 0, &UPMIX_PARAMS, lo)
                }
            }
            op::REQ_UPMIX_GET_CONFIG => self.field_bytes("upmix", 0, 0, ("config", 0, UPMIX_LEN)),
            op::REQ_UPMIX_GET_STATUS => {
                let on = self
                    .field_bytes("upmix", 0, 0, U_ENABLED)
                    .is_some_and(|b| b.first().is_some_and(|v| *v != 0));
                let st = UpmixStatus {
                    active: on,
                    // 0 active, 1 disabled.
                    parked_reason: u8::from(!on),
                    ..UpmixStatus::default()
                };
                Some(st.encode().to_vec())
            }
            op::REQ_GET_INPUT_SOURCE_NAME => {
                if value > 0xFF {
                    None
                } else {
                    self.extras
                        .source_names
                        .get(usize::from(lo))
                        .map(|n| n.to_vec())
                }
            }
            op::REQ_GET_DEVICE_NAME => (value == 0).then(|| self.extras.device_name.to_vec()),
            op::REQ_OUTPUT_LATENCY => match value {
                // -- WP TM20 --
                0 => self.latency_status_read(),
                1 => Some(self.latency_options()),
                _ => None,
            },
            op::REQ_GET_UART_CONFIG => Some(self.extras.uart.to_vec()),
            op::REQ_GET_I2C_CONFIG => Some(self.extras.i2c.to_vec()),
            // Write-as-read SETs: wValue in, a PIN_CONFIG_* status back.
            op::REQ_SET_ADAT_ENABLE => Some(vec![self.set_flag("adat_config", A_ENABLED, lo, 1)]),
            op::REQ_SET_MCK_ENABLE => Some(vec![self.set_flag(
                "i2s_config",
                I_MCK_ENABLED,
                u8::from(value != 0),
                1,
            )]),
            op::REQ_SET_MCK_MULTIPLIER => Some(vec![if value > 1 {
                pin::INVALID_PIN
            } else {
                self.set_flag("i2s_config", I_MCK_MULT, lo, 1)
            }]),
            op::REQ_SET_OUTPUT_TYPE => Some(vec![self.set_output_type(lo, hi)]),
            op::REQ_SET_I2S_INPUT_CHANNELS => Some(vec![self.set_i2s_channels(lo)]),
            op::REQ_SET_SPDIF_INPUT_ENABLE => Some(vec![self.set_spdif_enable(hi, lo != 0)]),
            op::REQ_SET_ADAT_INPUT_ENABLE => Some(vec![self.set_adat_input(lo)]),
            op::REQ_SET_ADAT_INPUT_CLOCK_MODE => Some(vec![if lo > 1 {
                pin::INVALID_PARAM
            } else {
                // Stored + 1 (0 = absent).
                self.put(
                    "input_config",
                    0,
                    0,
                    IC_ADAT_CLOCK_P1,
                    &[lo.saturating_add(1)],
                );
                pin::OK
            }]),
            _ => return None,
        };
        Some(r)
    }

    /// Feature OUT requests. `None`: not a feature opcode; `Some(false)`:
    /// stall.
    pub(super) fn set_feature(&mut self, opcode: u8, value: u16, data: &[u8]) -> Option<bool> {
        let [hi, lo] = value.to_be_bytes();
        let f32_arg = data
            .get(..4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(f32::from_le_bytes);
        let ok = match opcode {
            op::REQ_SET_INPUT_SOURCE => {
                if let Some(&src) = data.first()
                    && self.selectable(src)
                {
                    self.put("input_config", 0, 0, IC_SOURCE, &[src]);
                }
                true
            }
            op::REQ_SET_LEVELLER_MASKS => {
                // Two notifications, as the firmware sends them.
                if let [dm, am, ..] = *data {
                    self.put("leveller", 0, 0, L_DETECTOR, &[dm]);
                    self.put("leveller", 0, 0, L_APPLY, &[am]);
                }
                true
            }
            op::REQ_SET_INPUT_RATE => {
                let enc = match data.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()) {
                    Some(b) => match u32::from_le_bytes(b) {
                        44_100 => Some(0),
                        48_000 => Some(1),
                        96_000 => Some(2),
                        _ => None,
                    },
                    None => None,
                };
                if let Some(e) = enc {
                    self.put("input_config", 0, 0, IC_I2S_RATE, &[e]);
                }
                true
            }
            op::REQ_SET_DAC_HW_MUTE_CONFIG => {
                // Validated and applied later by the firmware; here at once.
                if let Some(cfg) = data.get(..DAC_HW_MUTE_LEN) {
                    self.put("dac_hw_mute", 0, 0, ("config", 0, DAC_HW_MUTE_LEN), cfg);
                }
                true
            }
            op::REQ_SET_TUBE_PARAM => {
                if let Some(v) = f32_arg
                    && !v.is_nan()
                    && let Some((f, num)) = TUBE_PARAMS.get(usize::from(lo))
                {
                    self.put("tube", 0, 0, *f, &num.encode(v));
                }
                true
            }
            op::REQ_LIMITER => {
                if let Some(v) = f32_arg {
                    self.set_limiter(hi, lo, v);
                }
                true
            }
            op::REQ_UPMIX_SET_PARAM => match (f32_arg, UPMIX_PARAMS.get(usize::from(value))) {
                (Some(v), Some((f, num))) => {
                    self.put("upmix", 0, 0, *f, &num.encode(v));
                    true
                }
                // An unknown parameter or a short payload is refused: ACKed
                // and dropped on USB, ERROR on UART/I2C/MIDI; stalls here
                // unless ack_unknown_sets.
                _ => false,
            },
            op::REQ_UPMIX_SET_CONFIG => {
                if data.len() != UPMIX_LEN {
                    // Refused: ACKed and dropped on USB, ERROR on
                    // UART/I2C/MIDI; stalls here unless ack_unknown_sets.
                    return Some(false);
                }
                // Field by field, one notification each, in struct order
                // (upmix_notify_config, upmix.c:234-256).
                let mut fields = UPMIX_PARAMS;
                fields.sort_by_key(|(f, _)| f.1);
                for (f, num) in fields {
                    if let Some(v) = data
                        .get(f.1..f.1.saturating_add(f.2))
                        .and_then(|b| num.decode(b))
                    {
                        self.put("upmix", 0, 0, f, &num.encode(v));
                    }
                }
                true
            }
            op::REQ_SET_INPUT_SOURCE_NAME => {
                if !data.is_empty() && value <= 0xFF {
                    self.set_source_name(lo, data);
                }
                true
            }
            op::REQ_SET_DEVICE_NAME => {
                if !data.is_empty() && value == 0 {
                    put_name(&mut self.extras.device_name, data);
                    let name = self.extras.device_name;
                    self.name_event(nfy::NOTIFY_EVT_DEVICE_NAME, 0, &name);
                }
                true
            }
            op::REQ_OUTPUT_LATENCY => match data.first() {
                // An unknown option is refused: ACKed and dropped on USB,
                // ERROR on UART/I2C/MIDI; stalls here unless
                // ack_unknown_sets.
                Some(&id) if LATENCY_OPTIONS.iter().any(|o| o.0 == id) => {
                    // -- WP TM20 --
                    self.latency_set(id);
                    true
                }
                _ => false,
            },
            op::REQ_SET_UART_CONFIG => {
                if let Some(b) = data.get(..UartCtrlConfig::SIZE)
                    && let Ok(cfg) = <[u8; UartCtrlConfig::SIZE]>::try_from(b)
                {
                    self.extras.uart = cfg;
                }
                true
            }
            op::REQ_SET_I2C_CONFIG => {
                if let Some(b) = data.get(..I2cCtrlConfig::SIZE)
                    && let Ok(cfg) = <[u8; I2cCtrlConfig::SIZE]>::try_from(b)
                {
                    // -- WP CG16 -- `i2c_ctrl_validate`: not the block a live
                    // display holds.
                    let display = self.surfaces.as_ref().and_then(|s| s.display_instance());
                    let clash = I2cCtrlConfig::decode(&cfg)
                        .is_ok_and(|c| c.enabled && display == Some((c.sda_pin >> 1) & 1));
                    if clash {
                        return Some(false);
                    }
                    self.extras.i2c = cfg;
                }
                true
            }
            _ => return None,
        };
        Some(ok)
    }

    fn get_param(
        &self,
        section: &str,
        stride: usize,
        i: usize,
        table: &[(Field, Num)],
        index: u8,
    ) -> Option<Vec<u8>> {
        let (f, num) = table.get(usize::from(index))?;
        let v = num.decode(&self.field_bytes(section, stride, i, *f)?)?;
        Some(v.to_le_bytes().to_vec())
    }

    /// `REQ_LIMITER` IN: `wValue = (output << 8) | index`; the meter and the
    /// status block are indexes `0x80` and `0x81`.
    fn get_limiter(&self, output: u8, index: u8) -> Option<Vec<u8>> {
        let outs = self.num_outputs();
        if u16::from(index) == lim::LIMITER_GET_METER {
            // No gain reduction anywhere.
            return Some(vec![0; usize::from(outs).saturating_mul(2)]);
        }
        if u16::from(index) == lim::LIMITER_GET_STATUS {
            // engaged, LIMITER_DELAY (2 * BLOCK), LIMITER_BLOCK, outputs.
            let block = lim::LIMITER_BLOCK as u8;
            return Some(vec![0, block.saturating_mul(2), block, outs]);
        }
        if output >= outs {
            return None;
        }
        self.get_param(
            "limiter",
            LIMITER_OUTPUT_LEN,
            usize::from(output),
            &LIMITER_PARAMS,
            index,
        )
    }

    /// `limiter_set_param` (limiter.c:172-194): every output for
    /// `LIMITER_ALL_OUTPUTS`; otherwise the output and the rest of its link
    /// group, and joining a group adopts the group's settings.
    fn set_limiter(&mut self, output: u8, index: u8, v: f32) {
        let outs = self.num_outputs();
        let Some((f, num)) = LIMITER_PARAMS.get(usize::from(index)).copied() else {
            return;
        };
        if v.is_nan() || (output >= outs && u16::from(output) != lim::LIMITER_ALL_OUTPUTS) {
            return;
        }
        let bytes = num.encode(v);
        let group = |s: &Self, k: u8| -> u8 {
            s.field_bytes(
                "limiter",
                LIMITER_OUTPUT_LEN,
                usize::from(k),
                LIMITER_OUTPUT_FIELDS[1],
            )
            .and_then(|b| b.first().copied())
            .unwrap_or(0)
        };
        let link = LIMITER_PARAMS[3].0;
        let targets: Vec<u8> = if u16::from(output) == lim::LIMITER_ALL_OUTPUTS {
            (0..outs).collect()
        } else if f == link {
            vec![output]
        } else {
            let g = group(self, output);
            (0..outs)
                .filter(|k| *k == output || (g != 0 && group(self, *k) == g))
                .collect()
        };
        for k in targets {
            self.put("limiter", LIMITER_OUTPUT_LEN, usize::from(k), f, &bytes);
        }
        if f == link && u16::from(output) != lim::LIMITER_ALL_OUTPUTS {
            let g = group(self, output);
            let peer = (0..outs).find(|k| *k != output && g != 0 && group(self, *k) == g);
            if let Some(p) = peer {
                for (pf, _) in LIMITER_PARAMS.iter().filter(|(pf, _)| *pf != link) {
                    if let Some(b) =
                        self.field_bytes("limiter", LIMITER_OUTPUT_LEN, usize::from(p), *pf)
                    {
                        self.put("limiter", LIMITER_OUTPUT_LEN, usize::from(output), *pf, &b);
                    }
                }
            }
        }
    }

    /// A 0/1 field set through a write-as-read; `value > max` is
    /// `INVALID_PARAM`.
    fn set_flag(&mut self, section: &str, f: Field, value: u8, max: u8) -> u8 {
        if value > max {
            return pin::INVALID_PARAM;
        }
        self.put(section, 0, 0, f, &[value]);
        pin::OK
    }

    /// `wValue = (type << 8) | slot`; a slot per stereo output pair.
    fn set_output_type(&mut self, slot: u8, kind: u8) -> u8 {
        if slot >= self.num_outputs() / 2 {
            return pin::INVALID_OUTPUT;
        }
        if kind > 1 {
            return pin::INVALID_PIN;
        }
        self.put(
            "i2s_config",
            1,
            usize::from(slot),
            ("output_type", 0, 1),
            &[kind],
        );
        pin::OK
    }

    fn set_i2s_channels(&mut self, count: u8) -> u8 {
        if !matches!(count, 2 | 4 | 6 | 8) {
            return pin::INVALID_PIN;
        }
        if count / 2 > i2s_pairs(self.header_byte(1)) {
            return pin::INVALID_OUTPUT;
        }
        self.put("input_config", 0, 0, IC_I2S_CHANNELS, &[count]);
        pin::OK
    }

    fn set_spdif_enable(&mut self, index: u8, enable: bool) -> u8 {
        if index == 0 {
            return if enable { pin::OK } else { pin::INVALID_OUTPUT };
        }
        if index >= SPDIF_INPUTS {
            return pin::INVALID_OUTPUT;
        }
        let bit = 1u8
            .checked_shl(u32::from(index.saturating_sub(1)))
            .unwrap_or(0);
        let mask = self.spdif_enabled_ext();
        let source = SRC_SPDIF2.saturating_add(index).saturating_sub(1);
        if !enable && self.ic(IC_SOURCE) == Some(source) {
            // The live source: switch away first.
            return pin::PIN_IN_USE;
        }
        let next = if enable { mask | bit } else { mask & !bit };
        if next != mask {
            self.put(
                "input_config",
                0,
                0,
                IC_SPDIF_ENABLED_P1,
                &[next.saturating_add(1)],
            );
        }
        pin::OK
    }

    fn set_adat_input(&mut self, value: u8) -> u8 {
        if value > 1 {
            return pin::INVALID_PARAM;
        }
        if value == 0 && self.ic(IC_SOURCE) == Some(SRC_ADAT) {
            return pin::PIN_IN_USE;
        }
        if value == 1 && matches!(self.ic(IC_ADAT_PIN), Some(0) | None) {
            // No pin configured.
            return pin::INVALID_PIN;
        }
        self.put(
            "input_config",
            0,
            0,
            IC_ADAT_ENABLED_P1,
            &[value.saturating_add(1)],
        );
        pin::OK
    }

    fn i2s_rx_pin(&self, pair: u8) -> Option<u8> {
        if pair == 0 {
            return self.ic(IC_I2S_RX_PIN);
        }
        let b = self.field_bytes("input_config", 0, 0, IC_I2S_RX_PIN_EXT)?;
        b.get(usize::from(pair.checked_sub(1)?)).copied()
    }

    fn spdif_pin(&self, index: u8) -> Option<u8> {
        if index == 0 {
            return self.ic(IC_SPDIF_PIN);
        }
        let b = self.field_bytes("input_config", 0, 0, IC_SPDIF_PIN_EXT)?;
        b.get(usize::from(index.checked_sub(1)?)).copied()
    }

    fn set_source_name(&mut self, src: u8, data: &[u8]) {
        let Some(n) = self.extras.source_names.get_mut(usize::from(src)) else {
            // An invalid source is a no-op.
            return;
        };
        put_name(n, data);
        let name = *n;
        self.name_event(nfy::NOTIFY_EVT_INPUT_SOURCE_NAME, src, &name);
    }

    // -- WP QD4 --
    /// Store a name as if someone else had set it: announced with a source
    /// that is no host's own. False for a source the device does not have, or
    /// an empty name field.
    pub(super) fn rename_elsewhere(&mut self, target: u8, data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }
        if target == super::RENAME_ELSEWHERE_DEVICE {
            put_name(&mut self.extras.device_name, data);
            let name = self.extras.device_name;
            self.name_event_from(
                nfy::NOTIFY_EVT_DEVICE_NAME,
                0,
                nfy::PARAM_SRC_GPIO as u8,
                &name,
            );
            return true;
        }
        let Some(n) = self.extras.source_names.get_mut(usize::from(target)) else {
            return false;
        };
        put_name(n, data);
        let name = *n;
        self.name_event_from(
            nfy::NOTIFY_EVT_INPUT_SOURCE_NAME,
            target,
            nfy::PARAM_SRC_GPIO as u8,
            &name,
        );
        true
    }

    /// `[2, evt, 0, seq, id, src, 0, 0, name(32)]` (notify.h:94-104). Sent on
    /// every store, changed or not.
    fn name_event(&mut self, evt: u16, id: u8, name: &[u8; NAME_LEN]) {
        self.name_event_from(evt, id, nfy::PARAM_SRC_HOST_SET as u8, name);
    }

    fn name_event_from(&mut self, evt: u16, id: u8, src: u8, name: &[u8; NAME_LEN]) {
        let mut p = vec![nfy::NOTIFY_V2_VERSION as u8, evt as u8, 0, 0, id, src, 0, 0];
        p.extend_from_slice(name);
        self.push(p);
    }

    // -- WP FIXS1 --
    /// The requested output latency option's `(latency_us, batch_us)` as the
    /// link input resolves it (`refresh_limits` in alink_input.c: source 7,
    /// so a stream-only option stands).
    pub(super) fn latency_for_alink(&self) -> (u16, u16) {
        let find = |id: u8| LATENCY_OPTIONS.iter().copied().find(|o| o.0 == id);
        find(self.extras.latency)
            .or_else(|| find(LATENCY_DEFAULT))
            .map_or((4000, 1000), |o| (o.2, o.3))
    }

    /// The option in force: a stream-only request on USB falls back to the
    /// first option without the flag (`output_latency_resolve`).
    fn latency_effective(&self) -> (u8, u8, u16, u16) {
        let find = |id: u8| LATENCY_OPTIONS.iter().copied().find(|o| o.0 == id);
        let opt = find(self.extras.latency)
            .or_else(|| find(LATENCY_DEFAULT))
            // Unreachable: the default is in the table.
            .unwrap_or((LATENCY_DEFAULT, 0, 4000, 1000));
        if opt.1 & 1 != 0 && self.ic(IC_SOURCE).unwrap_or(0) == 0 {
            return LATENCY_OPTIONS
                .iter()
                .copied()
                .find(|o| o.1 & 1 == 0)
                .unwrap_or(opt);
        }
        opt
    }

    /// `OutputLatencyStatus`, 12 bytes (config.h:1194-1205), at 48 kHz. The
    /// ring ceiling is not modelled: the target is the nominal latency.
    fn latency_status(&self) -> Vec<u8> {
        let (id, _, latency_us, batch_us) = self.latency_effective();
        let frames = |us: u16, up: u64| -> u16 {
            let n = u64::from(us)
                .saturating_mul(RATE_HZ)
                .saturating_add(up)
                .checked_div(1_000_000)
                .unwrap_or(0);
            u16::try_from(n).unwrap_or(u16::MAX)
        };
        let mut b = vec![self.extras.latency, id];
        b.extend(latency_us.to_le_bytes());
        b.extend(frames(latency_us, 500_000).to_le_bytes());
        b.extend(frames(batch_us, 999_999).to_le_bytes());
        // -- WP TM20 --
        b.push(u8::from(self.extras.latency_apply.pending.is_some()));
        b.push(LATENCY_OPTIONS.len() as u8);
        b.extend([0, 0]);
        b
    }

    /// `OutputLatencyOptionWire` per option, 8 bytes each.
    fn latency_options(&self) -> Vec<u8> {
        let usb = self.ic(IC_SOURCE).unwrap_or(0) == 0;
        let fallback = LATENCY_OPTIONS
            .iter()
            .find(|o| o.1 & 1 == 0)
            .map_or(0, |o| o.2);
        LATENCY_OPTIONS
            .iter()
            .flat_map(|&(id, flags, latency_us, batch_us)| {
                let runs = flags & 1 == 0 || !usb;
                let flags = flags | if runs { LATENCY_AVAILABLE } else { 0 };
                let effective = if runs { latency_us } else { fallback };
                let mut e = vec![id, flags];
                e.extend(latency_us.to_le_bytes());
                e.extend(batch_us.to_le_bytes());
                e.extend(effective.to_le_bytes());
                e
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t4b {
    use super::super::tests_wp_t4::{assert_contiguous, drain, f32_at, off_of};
    use super::super::*;
    use super::*;
    use dspi_proto::generated::{limiter, upmix as gup};

    fn len(s: &str) -> usize {
        section(s).unwrap().1
    }

    #[test]
    fn wp_t4b_layouts_fill_their_sections() {
        assert_contiguous(
            "leveller",
            &[
                L_ENABLED,
                L_SPEED,
                L_LOOKAHEAD,
                L_RESERVED,
                L_AMOUNT,
                L_MAX_GAIN,
                L_GATE,
                L_DETECTOR,
                L_APPLY,
                L_RESERVED2,
            ],
            len("leveller"),
        );
        assert_contiguous(
            "psybass",
            &[
                P_ENABLED,
                P_RESERVED,
                P_MASK,
                P_CUTOFF,
                P_HARMONICS,
                P_DRIVE,
                P_CHARACTER,
                P_ORIGINAL,
            ],
            len("psybass"),
        );
        let upmix = [
            U_ENABLED,
            U_CENTER,
            U_SURROUND,
            U_PRESENCE,
            U_STRENGTH,
            U_WIDTH,
            U_THRESHOLD,
            U_ATTACK,
            U_RELEASE,
            U_DET_HPF,
            U_SUR_DELAY,
            U_SUR_HPF,
            U_SUR_LPF,
            U_DECORR,
        ];
        assert_contiguous("upmix", &upmix, len("upmix"));
        assert_eq!(UPMIX_LEN, len("upmix"));
        assert_contiguous(
            "i2s_config",
            &[
                I_OUTPUT_TYPES,
                I_BCK,
                I_MCK_PIN,
                I_MCK_ENABLED,
                I_MCK_MULT,
                I_CLOCK_PIN_MODE,
                I_BCK_SLAVE,
                I_RESERVED,
            ],
            len("i2s_config"),
        );
        let mut lg = vec![LG_ENABLED];
        lg.extend(LG_REST);
        assert_contiguous("lg_sound_sync", &lg, len("lg_sound_sync"));
        assert_eq!(LG_LEN, len("lg_sound_sync"));
        assert_contiguous(
            "adat_config",
            &[A_ENABLED, A_PIN, A_RESERVED],
            len("adat_config"),
        );
        assert_eq!(DAC_HW_MUTE_LEN, len("dac_hw_mute"));
        // Fields borrowed from dspi_proto::wire by position: check the names.
        let names = |fs: &[Field]| fs.iter().map(|f| f.0).collect::<Vec<_>>();
        assert_eq!(
            names(&[
                IC_SOURCE,
                IC_SPDIF_PIN,
                IC_I2S_RX_PIN,
                IC_I2S_RATE,
                IC_I2S_CHANNELS,
                IC_I2S_RX_PIN_EXT,
                IC_SPDIF_PIN_EXT,
                IC_SPDIF_ENABLED_P1,
                IC_I2S_CLOCK_MODE,
                IC_ADAT_PIN,
                IC_ADAT_ENABLED_P1,
                IC_ADAT_CLOCK_P1
            ]),
            [
                "input_source",
                "spdif_rx_pin",
                "i2s_rx_pin",
                "i2s_input_rate",
                "i2s_input_channels",
                "i2s_rx_pin_ext",
                "spdif_rx_pin_ext",
                "spdif_rx_enabled_ext_p1",
                "i2s_clock_mode",
                "adat_input_pin",
                "adat_input_enabled_p1",
                "adat_clock_mode_p1"
            ]
        );
        assert_eq!(
            names(&[
                SH_ENABLED, SH_MASK, SH_LOW, SH_HIGH, SH_BOOST, SH_TOP, SH_DEPTH, SH_HOLD,
                SH_CEILING, SH_SELECT, SH_LINK
            ]),
            [
                "enabled",
                "output_mask",
                "low_db",
                "high_db",
                "boost_db",
                "top_db",
                "select_depth",
                "select_hold_ms",
                "ceiling_db",
                "select_mode",
                "link_pairs"
            ]
        );
        // The indexed tables, in the firmware's parameter order.
        let tube_names = [
            (tube::TUBE_PARAM_ENABLED, "enabled"),
            (tube::TUBE_PARAM_OUTPUT_MASK, "output_mask"),
            (tube::TUBE_PARAM_TUBE_TYPE, "tube_type"),
            (tube::TUBE_PARAM_DRIVE_DB, "drive_db"),
            (tube::TUBE_PARAM_BIAS_PCT, "bias_pct"),
            (tube::TUBE_PARAM_ASYM_DB, "asym_db"),
            (tube::TUBE_PARAM_HARDNESS_PCT, "hardness_pct"),
            (tube::TUBE_PARAM_SAG_PCT, "sag_pct"),
            (tube::TUBE_PARAM_RECTIFIER, "rectifier"),
            (tube::TUBE_PARAM_XFMR_ENABLED, "xfmr_enabled"),
            (tube::TUBE_PARAM_XFMR_DAMPING, "xfmr_damping"),
            (tube::TUBE_PARAM_XFMR_RES_HZ, "xfmr_res_hz"),
            (tube::TUBE_PARAM_MIX_PCT, "mix_pct"),
            (tube::TUBE_PARAM_TRIM_DB, "trim_db"),
        ];
        assert_eq!(TUBE_PARAMS.len(), tube::TUBE_NUM_PARAMS as usize);
        for (i, name) in tube_names {
            assert_eq!(TUBE_PARAMS[i as usize].0.0, name);
        }
        assert_eq!(LIMITER_PARAMS.len(), limiter::LIMITER_NUM_PARAMS as usize);
        for (i, name) in [
            (limiter::LIMITER_PARAM_ENABLED, "enabled"),
            (limiter::LIMITER_PARAM_THRESHOLD_DB, "threshold_db"),
            (limiter::LIMITER_PARAM_RELEASE_MS, "release_ms"),
            (limiter::LIMITER_PARAM_LINK_GROUP, "link_group"),
        ] {
            assert_eq!(LIMITER_PARAMS[i as usize].0.0, name);
        }
        // UPMIX_PARAM_* are not generated (an anonymous enum); upmix.h:188-201.
        let upmix_order = [
            "enabled",
            "center_mode",
            "surround_mode",
            "strength_pct",
            "center_width_pct",
            "corr_threshold_pct",
            "attack_ms",
            "release_ms",
            "detector_hpf_hz",
            "surround_delay_ms",
            "surround_hpf_hz",
            "surround_lpf_hz",
            "decorr_pct",
            "presence_q1",
        ];
        assert_eq!(names(&UPMIX_PARAMS.map(|p| p.0)), upmix_order);
        assert_eq!(gup::UPMIX_CENTER_OFF, 2);
        assert_eq!(gup::UPMIX_SURROUND_ADAPTIVE, 2);
        for s in FEATURE_SCALARS {
            let (_, slen) = section(s.section).unwrap();
            assert!(s.field.1 + s.field.2 <= slen, "{}.{}", s.section, s.field.0);
        }
    }

    #[test]
    fn wp_t4b_the_demo_answers_every_feature_probe() {
        use dspi_proto::generated::{limiter, opcodes as op, tube};
        // dspi_session::probe's feature list (probe.rs probe_features), minus
        // the signal generator and the spectrum analyser.
        let probes: &[(u8, u16, u16)] = &[
            (op::REQ_GET_INPUT_SOURCE, 0, 1),
            (op::REQ_GET_LOUDNESS_MASK, 0, 2),
            (op::REQ_GET_CROSSFEED_OUTPUTS, 0, 1),
            (op::REQ_GET_LEVELLER_MASKS, 0, 2),
            (op::REQ_GET_PSYBASS, 0, 1),
            (op::REQ_UPMIX_GET_STATUS, 0, 16),
            (op::REQ_GET_ADAT_ENABLE, 0, 1),
            (op::REQ_GET_ADAT_INPUT_ENABLE, 0, 1),
            (op::REQ_GET_I2S_CLOCK_MODE, 0, 1),
            (op::REQ_GET_I2S_INPUT_CHANNELS, 0, 1),
            (op::REQ_GET_SPDIF_INPUT_CONFIG, 0, 6),
            (op::REQ_GET_LG_SOUND_SYNC_ENABLE, 0, 1),
            (op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16),
            (op::REQ_GET_UART_CONFIG, 0, 8),
            (op::REQ_GET_I2C_CONFIG, 0, 8),
            (op::REQ_GET_SUBHARM, 0, 1),
            (op::REQ_GET_TUBE_PARAM, tube::TUBE_PARAM_ENABLED, 4),
            (op::REQ_LIMITER, limiter::LIMITER_GET_STATUS, 4),
            (op::REQ_GET_INPUT_SOURCE_NAME, 0, 32),
            (op::REQ_GET_DEVICE_NAME, 0, 32),
            (op::REQ_OUTPUT_LATENCY, 0, 12),
        ];
        let mut d = FakeDevice::demo();
        for &(opcode, value, n) in probes {
            let r = d.control_in(opcode, value, n);
            assert!(r.is_ok(), "0x{opcode:02X}: {r:?}");
        }
        assert_eq!(
            d.control_in(op::REQ_LIMITER, limiter::LIMITER_GET_STATUS, 4)
                .unwrap(),
            [0, 32, 16, 9]
        );
    }

    #[test]
    fn wp_t4b_scalars_round_trip_and_notify() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let f = |v: f32| v.to_le_bytes();
        let rows: &[(u8, u8, Vec<u8>)] = &[
            (
                op::REQ_SET_LEVELLER_AMOUNT,
                op::REQ_GET_LEVELLER_AMOUNT,
                f(42.0).to_vec(),
            ),
            (
                op::REQ_SET_PSYBASS_CUTOFF,
                op::REQ_GET_PSYBASS_CUTOFF,
                f(77.0).to_vec(),
            ),
            (
                op::REQ_SET_PSYBASS_MASK,
                op::REQ_GET_PSYBASS_MASK,
                vec![0x05, 0x01],
            ),
            (
                op::REQ_SET_SUBHARM_CEILING,
                op::REQ_GET_SUBHARM_CEILING,
                f(-6.5).to_vec(),
            ),
            (
                op::REQ_SET_SUBHARM_TOP,
                op::REQ_GET_SUBHARM_TOP,
                f(-11.25).to_vec(),
            ),
        ];
        for (set, get, v) in rows {
            d.control_out(*set, 0, v).unwrap();
            assert_eq!(&d.control_in(*get, 0, v.len() as u16).unwrap(), v);
        }
        assert_eq!(drain(&mut src).len(), rows.len());
        let sh = off_of("subharm");
        assert_eq!(f32_at(&d.remote().image(), sh + SH_CEILING.1), -6.5);

        // Booleans read back 0/1.
        d.control_out(op::REQ_SET_SUBHARM_LINK, 0, &[5]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_SUBHARM_LINK, 0, 1).unwrap(), [1]);

        // Clamps and ignores the firmware applies.
        d.control_out(op::REQ_SET_SUBHARM_SELECT, 0, &[9]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_SUBHARM_SELECT, 0, 1).unwrap(), [2]);
        let before = d.control_in(op::REQ_GET_LEVELLER_SPEED, 0, 1).unwrap();
        d.control_out(op::REQ_SET_LEVELLER_SPEED, 0, &[3]).unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_LEVELLER_SPEED, 0, 1).unwrap(),
            before
        );

        // The masks: one SET, two notifications.
        drain(&mut src);
        d.control_out(op::REQ_SET_LEVELLER_MASKS, 0, &[0x0F, 0xF0])
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_LEVELLER_MASKS, 0, 2).unwrap(),
            [0x0F, 0xF0]
        );
        let p = drain(&mut src);
        assert_eq!(p.len(), 2);
        let lv = off_of("leveller") as u16;
        assert_eq!(u16::from_le_bytes([p[0][4], p[0][5]]), lv + 16);
        assert_eq!(u16::from_le_bytes([p[1][4], p[1][5]]), lv + 17);
    }

    #[test]
    fn wp_t4b_tube_limiter_and_upmix_params() {
        let mut d = FakeDevice::demo();
        let f = |v: f32| v.to_le_bytes();
        let get = |d: &mut FakeDevice, opcode: u8, w: u16| {
            f32::from_le_bytes(d.control_in(opcode, w, 4).unwrap().try_into().unwrap())
        };
        // Tube: floats onto bytes, u16 and f32 fields.
        d.control_out(
            op::REQ_SET_TUBE_PARAM,
            tube::TUBE_PARAM_OUTPUT_MASK,
            &f(3.4),
        )
        .unwrap();
        assert_eq!(
            get(&mut d, op::REQ_GET_TUBE_PARAM, tube::TUBE_PARAM_OUTPUT_MASK),
            3.0
        );
        d.control_out(op::REQ_SET_TUBE_PARAM, tube::TUBE_PARAM_RECTIFIER, &f(9.0))
            .unwrap();
        assert_eq!(
            get(&mut d, op::REQ_GET_TUBE_PARAM, tube::TUBE_PARAM_RECTIFIER),
            3.0
        );
        d.control_out(op::REQ_SET_TUBE_PARAM, tube::TUBE_PARAM_DRIVE_DB, &f(6.5))
            .unwrap();
        assert_eq!(
            get(&mut d, op::REQ_GET_TUBE_PARAM, tube::TUBE_PARAM_DRIVE_DB),
            6.5
        );
        assert!(d.control_in(op::REQ_GET_TUBE_PARAM, 14, 4).is_err());

        // Limiter: one output, all outputs, link groups.
        let w = |out: u16, idx: u16| (out << 8) | idx;
        let thr = limiter::LIMITER_PARAM_THRESHOLD_DB;
        let grp = limiter::LIMITER_PARAM_LINK_GROUP;
        d.control_out(op::REQ_LIMITER, w(0xFF, thr), &f(-3.0))
            .unwrap();
        for k in 0..9 {
            assert_eq!(get(&mut d, op::REQ_LIMITER, w(k, thr)), -3.0);
        }
        d.control_out(op::REQ_LIMITER, w(1, thr), &f(-9.0)).unwrap();
        d.control_out(op::REQ_LIMITER, w(1, grp), &f(2.0)).unwrap();
        // Output 4 joins group 2 and adopts output 1's threshold.
        d.control_out(op::REQ_LIMITER, w(4, grp), &f(2.0)).unwrap();
        assert_eq!(get(&mut d, op::REQ_LIMITER, w(4, thr)), -9.0);
        // A change on one member reaches the group.
        d.control_out(op::REQ_LIMITER, w(4, thr), &f(-12.0))
            .unwrap();
        assert_eq!(get(&mut d, op::REQ_LIMITER, w(1, thr)), -12.0);
        assert_eq!(get(&mut d, op::REQ_LIMITER, w(0, thr)), -3.0);
        assert!(d.control_in(op::REQ_LIMITER, w(9, thr), 4).is_err());
        assert_eq!(
            d.control_in(op::REQ_LIMITER, limiter::LIMITER_GET_METER, 18)
                .unwrap(),
            [0; 18]
        );

        // Upmix: by parameter, and the whole packet.
        d.control_out(op::REQ_UPMIX_SET_PARAM, 13, &f(-3.5))
            .unwrap();
        assert_eq!(get(&mut d, op::REQ_UPMIX_GET_PARAM, 13), -3.5);
        assert_eq!(d.remote().image()[off_of("upmix") + 3] as i8, -7);
        assert!(d.control_out(op::REQ_UPMIX_SET_PARAM, 14, &f(1.0)).is_err());
        assert!(d.control_out(op::REQ_UPMIX_SET_PARAM, 0, &[1]).is_err());
        let mut cfg = d.control_in(op::REQ_UPMIX_GET_CONFIG, 0, 44).unwrap();
        cfg[0] = 1;
        cfg[4..8].copy_from_slice(&f(80.0));
        d.control_out(op::REQ_UPMIX_SET_CONFIG, 0, &cfg).unwrap();
        assert_eq!(d.control_in(op::REQ_UPMIX_GET_CONFIG, 0, 44).unwrap(), cfg);
        assert_eq!(
            d.control_in(op::REQ_UPMIX_GET_STATUS, 0, 16).unwrap()[..2],
            [1, 0]
        );
        assert!(
            d.control_out(op::REQ_UPMIX_SET_CONFIG, 0, &cfg[..40])
                .is_err()
        );
    }

    #[test]
    fn wp_t4b_inputs_clocks_and_outputs() {
        let mut d = FakeDevice::demo();
        let st = |d: &mut FakeDevice, opcode: u8, w: u16| d.control_in(opcode, w, 1).unwrap()[0];
        // S/PDIF 2 can only be selected once enabled.
        d.control_out(op::REQ_SET_INPUT_SOURCE, 0, &[SRC_SPDIF2])
            .unwrap();
        assert_ne!(st(&mut d, op::REQ_GET_INPUT_SOURCE, 0), SRC_SPDIF2);
        assert_eq!(
            st(&mut d, op::REQ_SET_SPDIF_INPUT_ENABLE, (1 << 8) | 1),
            pin::OK
        );
        assert_eq!(
            d.control_in(op::REQ_GET_SPDIF_INPUT_CONFIG, 0, 6).unwrap()[..2],
            [4, 0b11]
        );
        d.control_out(op::REQ_SET_INPUT_SOURCE, 0, &[SRC_SPDIF2])
            .unwrap();
        assert_eq!(st(&mut d, op::REQ_GET_INPUT_SOURCE, 0), SRC_SPDIF2);
        // ...and not disabled while it is the source.
        assert_eq!(
            st(&mut d, op::REQ_SET_SPDIF_INPUT_ENABLE, 1 << 8),
            pin::PIN_IN_USE
        );
        assert_eq!(
            st(&mut d, op::REQ_SET_SPDIF_INPUT_ENABLE, 0),
            pin::INVALID_OUTPUT
        );

        assert_eq!(st(&mut d, op::REQ_SET_I2S_INPUT_CHANNELS, 6), pin::OK);
        assert_eq!(st(&mut d, op::REQ_GET_I2S_INPUT_CHANNELS, 0), 6);
        assert_eq!(
            st(&mut d, op::REQ_SET_I2S_INPUT_CHANNELS, 5),
            pin::INVALID_PIN
        );
        d.control_out(op::REQ_SET_I2S_CLOCK_MODE, 0, &[1]).unwrap();
        d.control_out(op::REQ_SET_I2S_CLOCK_MODE, 0, &[2]).unwrap();
        assert_eq!(st(&mut d, op::REQ_GET_I2S_CLOCK_MODE, 0), 1);
        d.control_out(op::REQ_SET_INPUT_RATE, 0, &96_000u32.to_le_bytes())
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_INPUT_RATE, 0, 8).unwrap()[4..],
            96_000u32.to_le_bytes()
        );

        assert_eq!(st(&mut d, op::REQ_SET_ADAT_ENABLE, 1), pin::OK);
        assert_eq!(st(&mut d, op::REQ_GET_ADAT_ENABLE, 0), 1);
        assert_eq!(st(&mut d, op::REQ_SET_ADAT_ENABLE, 2), pin::INVALID_PARAM);
        assert_eq!(st(&mut d, op::REQ_SET_ADAT_INPUT_CLOCK_MODE, 1), pin::OK);
        assert_eq!(st(&mut d, op::REQ_GET_ADAT_INPUT_CLOCK_MODE, 0), 1);

        assert_eq!(st(&mut d, op::REQ_SET_OUTPUT_TYPE, (1 << 8) | 2), pin::OK);
        assert_eq!(st(&mut d, op::REQ_GET_OUTPUT_TYPE, 2), 1);
        assert_eq!(
            st(&mut d, op::REQ_SET_OUTPUT_TYPE, (1 << 8) | 4),
            pin::INVALID_OUTPUT
        );
        assert!(d.control_in(op::REQ_GET_OUTPUT_TYPE, 4, 1).is_err());
        assert_eq!(st(&mut d, op::REQ_SET_MCK_ENABLE, 1), pin::OK);
        assert_eq!(st(&mut d, op::REQ_GET_MCK_ENABLE, 0), 1);
        assert_eq!(st(&mut d, op::REQ_SET_MCK_MULTIPLIER, 7), pin::INVALID_PIN);

        let mut cfg = d.control_in(op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16).unwrap();
        cfg[0] = 1;
        cfg[2] = 22;
        d.control_out(op::REQ_SET_DAC_HW_MUTE_CONFIG, 0, &cfg)
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16).unwrap(),
            cfg
        );
        d.control_out(op::REQ_SET_LG_SOUND_SYNC_ENABLE, 0, &[1])
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_LG_SOUND_SYNC_STATUS, 0, 16)
                .unwrap()[0],
            1
        );
    }

    #[test]
    fn wp_t4b_names_latency_and_control_interfaces() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        d.control_out(op::REQ_SET_INPUT_SOURCE_NAME, 2, b"Turntable")
            .unwrap();
        let n = d.control_in(op::REQ_GET_INPUT_SOURCE_NAME, 2, 32).unwrap();
        assert_eq!(&n[..10], b"Turntable\0");
        assert!(d.control_in(op::REQ_GET_INPUT_SOURCE_NAME, 7, 32).is_err());
        assert!(
            d.control_in(op::REQ_GET_INPUT_SOURCE_NAME, 0x100, 32)
                .is_err()
        );
        d.control_out(op::REQ_SET_DEVICE_NAME, 0, b"Den").unwrap();
        assert_eq!(
            &d.control_in(op::REQ_GET_DEVICE_NAME, 0, 32).unwrap()[..4],
            b"Den\0"
        );
        // A leading NUL clears.
        d.control_out(op::REQ_SET_DEVICE_NAME, 0, &[0, b'x'])
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_DEVICE_NAME, 0, 32).unwrap(),
            [0; 32]
        );
        let p = drain(&mut src);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0][1], p[0][4], p[0].len()), (0x0F, 2, 40));
        assert_eq!(&p[0][8..17], b"Turntable");
        assert_eq!((p[1][1], p[1][4]), (0x10, 0));
        assert_eq!(p[1][5], nfy::PARAM_SRC_HOST_SET as u8);

        // Latency: the default 4 ms, then a stream-only option on USB.
        let s = d.control_in(op::REQ_OUTPUT_LATENCY, 0, 12).unwrap();
        assert_eq!(s[..4], [5, 5, 0xA0, 0x0F]);
        assert_eq!(s[9], 7);
        d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[1]).unwrap();
        let s = d.control_in(op::REQ_OUTPUT_LATENCY, 0, 12).unwrap();
        assert_eq!(
            s[..2],
            [1, 3],
            "0.67 ms needs a stream input; USB runs 2 ms"
        );
        assert!(d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[9]).is_err());
        let opts = d.control_in(op::REQ_OUTPUT_LATENCY, 1, 56).unwrap();
        assert_eq!(opts[..2], [1, 1]);
        assert_eq!(opts[16..18], [3, 0x80]);
        assert!(d.control_in(op::REQ_OUTPUT_LATENCY, 2, 1).is_err());

        let uart = UartCtrlConfig {
            baud: 9600,
            ..UartCtrlConfig::default()
        };
        d.control_out(op::REQ_SET_UART_CONFIG, 0, &uart.encode())
            .unwrap();
        assert_eq!(
            UartCtrlConfig::decode(&d.control_in(op::REQ_GET_UART_CONFIG, 0, 8).unwrap()).unwrap(),
            uart
        );
        assert_eq!(
            I2cCtrlConfig::decode(&d.control_in(op::REQ_GET_I2C_CONFIG, 0, 8).unwrap()).unwrap(),
            I2cCtrlConfig::default()
        );
    }

    #[test]
    fn wp_t4c_upmix_clamps_and_notifies_in_struct_order() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let f = |v: f32| v.to_le_bytes();
        let up = off_of("upmix");
        // Presence: +-12 dB, so the half-dB byte stops at 24.
        d.control_out(op::REQ_UPMIX_SET_PARAM, 13, &f(20.0))
            .unwrap();
        assert_eq!(d.remote().image()[up + 3], 24);
        d.control_out(op::REQ_UPMIX_SET_PARAM, 13, &f(-20.0))
            .unwrap();
        assert_eq!(d.remote().image()[up + 3] as i8, -24);
        // lrintf: ties to even.
        d.control_out(op::REQ_UPMIX_SET_PARAM, 13, &f(1.25))
            .unwrap();
        assert_eq!(d.remote().image()[up + 3], 2);
        // An out-of-range centre mode falls back to adaptive (1); surround
        // clamps to 2.
        d.control_out(op::REQ_UPMIX_SET_PARAM, 1, &f(0.0)).unwrap();
        d.control_out(op::REQ_UPMIX_SET_PARAM, 1, &f(7.0)).unwrap();
        assert_eq!(d.remote().image()[up + 1], 1);
        d.control_out(op::REQ_UPMIX_SET_PARAM, 1, &f(-1.0)).unwrap();
        assert_eq!(d.remote().image()[up + 1], 1);
        d.control_out(op::REQ_UPMIX_SET_PARAM, 2, &f(7.0)).unwrap();
        assert_eq!(d.remote().image()[up + 2], 2);
        // Ties to even, as lrintf: 0.5 -> 0, 1.5 -> 2.
        d.control_out(op::REQ_UPMIX_SET_PARAM, 2, &f(0.5)).unwrap();
        assert_eq!(d.remote().image()[up + 2], 0);
        d.control_out(op::REQ_UPMIX_SET_PARAM, 2, &f(1.5)).unwrap();
        assert_eq!(d.remote().image()[up + 2], 2);

        // SET_CONFIG: one notification per changed field, in struct order.
        drain(&mut src);
        let mut cfg = d.control_in(op::REQ_UPMIX_GET_CONFIG, 0, 44).unwrap();
        cfg[1] = 9; // centre mode out of range
        cfg[3] = 100; // presence +50 dB
        cfg[40..44].copy_from_slice(&f(33.0));
        cfg[4..8].copy_from_slice(&f(61.0));
        d.control_out(op::REQ_UPMIX_SET_CONFIG, 0, &cfg).unwrap();
        let img = d.remote().image();
        assert_eq!((img[up + 1], img[up + 3]), (1, 24));
        let offs: Vec<usize> = drain(&mut src)
            .iter()
            .map(|p| usize::from(u16::from_le_bytes([p[4], p[5]])) - up)
            .collect();
        assert_eq!(offs, [3, 4, 40], "centre was already 1");
    }

    #[test]
    fn wp_t4c_refusals_ack_with_ack_unknown_sets() {
        let mut d = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                ack_unknown_sets: true,
                ..FakeOptions::default()
            },
        );
        let before = d.remote().image();
        d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[9]).unwrap();
        assert_eq!(d.control_in(op::REQ_OUTPUT_LATENCY, 0, 12).unwrap()[0], 5);
        d.control_out(op::REQ_UPMIX_SET_PARAM, 14, &1.0f32.to_le_bytes())
            .unwrap();
        d.control_out(op::REQ_UPMIX_SET_PARAM, 0, &[1]).unwrap();
        d.control_out(op::REQ_UPMIX_SET_CONFIG, 0, &[1; 40])
            .unwrap();
        let mut bad = before.clone();
        bad[0] = 31;
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &bad).unwrap();
        assert_eq!(d.remote().image(), before, "acknowledged, not applied");
        // A chunk out of sequence and the extension channel still stall.
        assert!(
            d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, 2048, &[0; 16])
                .is_err()
        );
        assert!(d.control_out(op::REQ_EXTENSION, 0x7777, &[0; 4]).is_err());
        assert!(d.control_out(op::REQ_EXTENSION, 2, &[9, 0]).is_err());
    }
}
