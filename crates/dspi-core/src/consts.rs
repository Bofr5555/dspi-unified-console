//! Named values for the raw codes the records carry (`COMPAT_*`,
//! `NOTIFY_*` flags of `process_notifications`, ...), where the firmware
//! headers (`dspi_proto::generated`) do not already name them.
//!
//! The old desktop core's names are kept, so the C header and the Kotlin
//! `DspiConstants` keep matching. Shared file: add constants, never change
//! one (reviewed per wave).

/// `DspState.compat`: not judged yet.
pub const COMPAT_UNKNOWN: u8 = 0;
/// Wire format and firmware match this core.
pub const COMPAT_OK: u8 = 1;
/// Firmware older than this core's wire format (shown read-only).
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;
/// Firmware newer than this core's wire format (shown read-only).
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

// Work-package sections: append only inside your own section.
// -- WP C0 --

/// Wire channels (inputs + outputs) the V32 image carries; the length of
/// every wire-channel-indexed `DspState` vector.
pub const MAX_CHANNELS: usize = dspi_proto::generated::wire::WIRE_MAX_CHANNELS as usize;
/// Input channels the V32 image carries (`DspState.input_preamp_db`, matrix
/// rows).
pub const MAX_INPUTS: usize = dspi_proto::generated::wire::WIRE_MAX_INPUT_CHANNELS as usize;
/// Output channels the V32 image carries (output-indexed vectors, matrix
/// columns).
pub const MAX_OUTPUTS: usize = dspi_proto::generated::wire::WIRE_MAX_OUTPUT_CHANNELS as usize;
/// PEQ bands per channel the firmware processes (`DspState.filters` rows).
pub const BANDS_PER_CHANNEL: usize = 10;
/// PEQ band slots per channel in the wire layout (the last two are unused).
pub const FIRMWARE_BANDS_PER_CHANNEL: usize = dspi_proto::generated::wire::WIRE_MAX_BANDS as usize;
/// Crossover bands per channel (`DspState.xover` rows).
pub const MAX_XOVER_BANDS: usize = dspi_proto::generated::wire::WIRE_MAX_XOVER_BANDS as usize;
/// Preset slots (`DspState.preset_names`).
pub const MAX_PRESETS: usize = 10;
/// Channel and preset name buffer, bytes, NUL included: names hold at most
/// 31 bytes of UTF-8.
pub const CHANNEL_NAME_LEN: usize = dspi_proto::generated::wire::WIRE_NAME_LEN as usize;
/// Physical output pin slots (`DspState.output_pins`).
pub const MAX_PHYSICAL_OUTPUTS: usize = dspi_proto::generated::wire::WIRE_MAX_PIN_OUTPUTS as usize;
/// Crossfeed output pairs in `DspState.crossfeed_pair_map`.
pub const CROSSFEED_MAP_PAIRS: usize = 4;
/// Default (Butterworth) Q of a filter band, also the Linkwitz Transform
/// target-Q default.
pub const DEFAULT_Q: f32 = 0.707;

/// `DspState.fw_beta` for a beta that cannot say which one it is: the 1.1.6
/// betas 1 and 2 answer `REQ_GET_PLATFORM` without the beta byte. Sorts
/// below every numbered beta.
pub const FW_BETA_EARLY: u8 = 0xFF;

/// `DspState.upmix` index: enable (0/1).
pub const UPMIX_PARAM_ENABLED: u8 = 0;
/// `DspState.upmix` index: centre mode (0 passive, 1 adaptive, 2 off).
pub const UPMIX_PARAM_CENTER_MODE: u8 = 1;
/// `DspState.upmix` index: surround mode (0 off, 1 passive, 2 adaptive).
pub const UPMIX_PARAM_SURROUND_MODE: u8 = 2;
/// `DspState.upmix` index: strength, %.
pub const UPMIX_PARAM_STRENGTH: u8 = 3;
/// `DspState.upmix` index: width, %.
pub const UPMIX_PARAM_WIDTH: u8 = 4;
/// `DspState.upmix` index: threshold, %.
pub const UPMIX_PARAM_THRESHOLD: u8 = 5;
/// `DspState.upmix` index: attack, ms.
pub const UPMIX_PARAM_ATTACK: u8 = 6;
/// `DspState.upmix` index: release, ms.
pub const UPMIX_PARAM_RELEASE: u8 = 7;
/// `DspState.upmix` index: detector high-pass, Hz.
pub const UPMIX_PARAM_DETECTOR_HPF: u8 = 8;
/// `DspState.upmix` index: surround delay, ms.
pub const UPMIX_PARAM_SURROUND_DELAY: u8 = 9;
/// `DspState.upmix` index: surround high-pass, Hz.
pub const UPMIX_PARAM_SURROUND_HPF: u8 = 10;
/// `DspState.upmix` index: surround low-pass, Hz.
pub const UPMIX_PARAM_SURROUND_LPF: u8 = 11;
/// `DspState.upmix` index: decorrelation, %.
pub const UPMIX_PARAM_DECORRELATION: u8 = 12;
/// `DspState.upmix` index: presence bell, dB (0.5 dB steps on the wire).
pub const UPMIX_PARAM_PRESENCE: u8 = 13;
/// Length of `DspState.upmix`.
pub const UPMIX_PARAM_COUNT: usize = 14;

/// `DspState.subharm` index: enable (0/1).
pub const SUBHARM_PARAM_ENABLED: u8 = 0;
/// `DspState.subharm` index: output mask (bit k = output k).
pub const SUBHARM_PARAM_MASK: u8 = 1;
/// `DspState.subharm` index: low band level, dB.
pub const SUBHARM_PARAM_LOW: u8 = 2;
/// `DspState.subharm` index: high band level, dB.
pub const SUBHARM_PARAM_HIGH: u8 = 3;
/// `DspState.subharm` index: top band level, dB.
pub const SUBHARM_PARAM_TOP: u8 = 4;
/// `DspState.subharm` index: boost, dB.
pub const SUBHARM_PARAM_BOOST: u8 = 5;
/// `DspState.subharm` index: selectivity mode (0 all, 1 percussive, 2 sustained).
pub const SUBHARM_PARAM_SELECT: u8 = 6;
/// `DspState.subharm` index: selectivity depth, %.
pub const SUBHARM_PARAM_DEPTH: u8 = 7;
/// `DspState.subharm` index: selectivity hold, ms.
pub const SUBHARM_PARAM_HOLD: u8 = 8;
/// `DspState.subharm` index: sub ceiling, dB.
pub const SUBHARM_PARAM_CEILING: u8 = 9;
/// `DspState.subharm` index: pair link (0/1).
pub const SUBHARM_PARAM_LINK: u8 = 10;
/// Length of `DspState.subharm`.
pub const SUBHARM_PARAM_COUNT: usize = 11;

/// Length of `DspState.tube`, indexed by the firmware's `TUBE_PARAM_*`
/// (`dspi_proto::generated::tube`).
pub const TUBE_PARAM_COUNT: usize = dspi_proto::generated::tube::TUBE_NUM_PARAMS as usize;
// -- WP C1 --
// -- WP C1b --
// -- WP C2 --
// -- WP C3 --
// -- WP C4 --
// -- WP C5 --
// -- WP C6 --
// -- WP C7 --
// -- WP C8 --
// -- WP C9 --
// -- WP C10 --
// -- WP C11 --
// -- WP C12 --
// -- WP C13 --
// -- WP C14 --
// -- WP C15 --
// -- WP F1 --
