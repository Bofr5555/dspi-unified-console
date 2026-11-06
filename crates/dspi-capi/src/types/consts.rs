//! The named constants of the C header, copied value for value from the old
//! desktop core (`crates/dspi-core-linux`, every `pub const` cbindgen put
//! into its `dspi_core.h`), so the Qt app compiles unchanged (WP Q1).
//!
//! cbindgen only sees literals, so these stay literals. The tests in
//! `types.rs` check them against the firmware-generated constants
//! (`dspi_proto::generated`) and `dspi_core::consts` wherever those name the
//! same thing, and the header test checks every `#define` against the old
//! header. Grouped by the old core's module.

// ── dsp_math.rs ────────────────────────────────────────────────────

pub const SAMPLE_RATE: f64 = 48000.0;
/// Number of magnitude curve points (log-spaced from 10 Hz to 20 kHz).
pub const MAGNITUDE_POINTS: usize = 201;

// ── notify.rs ──────────────────────────────────────────────────────

/// Bulk IN notification endpoint on the vendor interface.
pub const NOTIFY_ENDPOINT: u8 = 0x83;
/// `NotifyResult.flags`: some parameter changed; re-read the state.
pub const NOTIFY_STATE: u32 = 1 << 0;
/// A change that moves the graph (filters, gains, bypass, output enable).
pub const NOTIFY_CURVES: u32 = 1 << 1;
/// Everything was re-read from the device (preset load, bulk SET, missed packets).
pub const NOTIFY_REFRESHED: u32 = 1 << 2;
/// The active USB input channel count changed.
pub const NOTIFY_INPUT_FORMAT: u32 = 1 << 3;
/// A preset was loaded on the device.
pub const NOTIFY_PRESET: u32 = 1 << 4;
/// The signal generator started or stopped (re-read its status). Not a
/// parameter change: comes without NOTIFY_STATE.
pub const NOTIFY_SIGGEN: u32 = 1 << 5;
/// An auxiliary output changed state or level (already in the control
/// surfaces snapshot).
pub const NOTIFY_CS_AUX: u32 = 1 << 6;
/// IR learning finished: a code was captured or the window timed out
/// (the result is in the control surfaces snapshot).
pub const NOTIFY_IR_LEARN: u32 = 1 << 7;

// ── protocol.rs ────────────────────────────────────────────────────

/// DSPi vendor ID. Firmware moved from 0x2E8A (Raspberry Pi's, still used by
/// the RP2040/RP2350 ROM bootloader) to 0x2E8B in May 2026.
pub const VENDOR_ID: u16 = 0x2e8b;
/// Vendor ID used by firmware before the change; still recognised so an old
/// device shows up and can be told it needs a firmware update.
pub const LEGACY_VENDOR_ID: u16 = 0x2e8a;
pub const PRODUCT_ID: u16 = 0xfeaa;
/// Vendor interface number (WinUSB / WCID interface).
pub const VENDOR_INTERFACE: u8 = 2;
/// Host→Device | Vendor | Interface
pub const REQ_TYPE_OUT: u8 = 0x41;
/// Device→Host | Vendor | Interface
pub const REQ_TYPE_IN: u8 = 0xC1;
pub const REQ_SET_CS_AUX_STATE: u8 = 0x04;
pub const REQ_GET_CS_AUX_STATE: u8 = 0x05;
pub const REQ_SET_CS_AUX_LEVEL: u8 = 0x06;
pub const REQ_GET_CS_AUX_LEVEL: u8 = 0x07;
pub const REQ_RTA_SET_CONFIG: u8 = 0x08;
pub const REQ_RTA_GET_CONFIG: u8 = 0x09;
pub const REQ_RTA_GET_CAPS: u8 = 0x0A;
pub const REQ_RTA_GET_BANDS: u8 = 0x0B;
pub const REQ_RTA_GET_BINS: u8 = 0x0C;
pub const REQ_RTA_GET_STATUS: u8 = 0x0D;
pub const REQ_RTA_CONTROL: u8 = 0x0E;
pub const REQ_RTA_GET_BANDS_ALL: u8 = 0x0F;
pub const REQ_SET_SUBHARM: u8 = 0x10;
pub const REQ_GET_SUBHARM: u8 = 0x11;
pub const REQ_SET_SUBHARM_LOW: u8 = 0x12;
pub const REQ_GET_SUBHARM_LOW: u8 = 0x13;
pub const REQ_SET_SUBHARM_HIGH: u8 = 0x14;
pub const REQ_GET_SUBHARM_HIGH: u8 = 0x15;
pub const REQ_SET_SUBHARM_BOOST: u8 = 0x16;
pub const REQ_GET_SUBHARM_BOOST: u8 = 0x17;
pub const REQ_SET_SUBHARM_MASK: u8 = 0x18;
pub const REQ_GET_SUBHARM_MASK: u8 = 0x19;
pub const REQ_GET_SUBHARM_HEADROOM: u8 = 0x1A;
pub const REQ_SET_SUBHARM_TOP: u8 = 0x1B;
pub const REQ_GET_SUBHARM_TOP: u8 = 0x1C;
pub const REQ_SET_SUBHARM_SELECT: u8 = 0x1D;
pub const REQ_GET_SUBHARM_SELECT: u8 = 0x1E;
pub const REQ_GET_SUBHARM_METER: u8 = 0x1F;
pub const REQ_SET_SUBHARM_SOLO: u8 = 0x2C;
pub const REQ_GET_SUBHARM_SOLO: u8 = 0x2D;
pub const REQ_SET_SUBHARM_LINK: u8 = 0x2E;
pub const REQ_GET_SUBHARM_LINK: u8 = 0x2F;
pub const REQ_SET_SUBHARM_DEPTH: u8 = 0xA9;
pub const REQ_GET_SUBHARM_DEPTH: u8 = 0xAA;
pub const REQ_SET_SUBHARM_HOLD: u8 = 0xAB;
pub const REQ_GET_SUBHARM_HOLD: u8 = 0xAC;
pub const REQ_SET_SUBHARM_CEILING: u8 = 0xAD;
pub const REQ_GET_SUBHARM_CEILING: u8 = 0xAE;
pub const REQ_SET_CS_GROUP: u8 = 0x20;
pub const REQ_GET_CS_GROUP: u8 = 0x21;
pub const REQ_SET_CS_MACRO: u8 = 0x22;
pub const REQ_GET_CS_MACRO: u8 = 0x23;
pub const REQ_SET_CS_MACRO_STEP: u8 = 0x24;
pub const REQ_CS_MACRO_FIRE: u8 = 0x25;
pub const REQ_GET_CS_EXT_STATUS: u8 = 0x26;
pub const REQ_SET_CS_DISPLAY_CFG: u8 = 0x27;
pub const REQ_GET_CS_DISPLAY_CFG: u8 = 0x28;
pub const REQ_SET_CS_DISPLAY_PAGE: u8 = 0x29;
pub const REQ_GET_CS_DISPLAY_PAGE: u8 = 0x2A;
pub const REQ_GET_CS_DISPLAY_STATUS: u8 = 0x2B;
pub const REQ_SET_PSYBASS: u8 = 0x30;
pub const REQ_GET_PSYBASS: u8 = 0x31;
pub const REQ_SET_PSYBASS_CUTOFF: u8 = 0x32;
pub const REQ_GET_PSYBASS_CUTOFF: u8 = 0x33;
pub const REQ_SET_PSYBASS_HARMONICS: u8 = 0x34;
pub const REQ_GET_PSYBASS_HARMONICS: u8 = 0x35;
pub const REQ_SET_PSYBASS_DRIVE: u8 = 0x36;
pub const REQ_GET_PSYBASS_DRIVE: u8 = 0x37;
pub const REQ_SET_PSYBASS_CHARACTER: u8 = 0x38;
pub const REQ_GET_PSYBASS_CHARACTER: u8 = 0x39;
pub const REQ_SET_PSYBASS_ORIGINAL: u8 = 0x3A;
pub const REQ_GET_PSYBASS_ORIGINAL: u8 = 0x3B;
pub const REQ_SET_PSYBASS_MASK: u8 = 0x3C;
pub const REQ_GET_PSYBASS_MASK: u8 = 0x3D;
/// Psybass parameter ids for `set_psybass_param` (host-side, not wire values).
pub const PSYBASS_PARAM_CUTOFF: u8 = 0;
pub const PSYBASS_PARAM_HARMONICS: u8 = 1;
pub const PSYBASS_PARAM_DRIVE: u8 = 2;
pub const PSYBASS_PARAM_CHARACTER: u8 = 3;
pub const PSYBASS_PARAM_ORIGINAL: u8 = 4;
pub const REQ_SET_TUBE_PARAM: u8 = 0x3E;
pub const REQ_GET_TUBE_PARAM: u8 = 0x3F;
/// Tube parameter indices (the firmware's own; wValue of REQ_SET_TUBE_PARAM).
/// Every value travels as a float, enums and flags included.
pub const TUBE_PARAM_ENABLED: u8 = 0;
pub const TUBE_PARAM_MASK: u8 = 1;
pub const TUBE_PARAM_TYPE: u8 = 2;
pub const TUBE_PARAM_DRIVE: u8 = 3;
pub const TUBE_PARAM_BIAS: u8 = 4;
pub const TUBE_PARAM_ASYM: u8 = 5;
pub const TUBE_PARAM_HARDNESS: u8 = 6;
pub const TUBE_PARAM_SAG: u8 = 7;
pub const TUBE_PARAM_RECTIFIER: u8 = 8;
pub const TUBE_PARAM_XFMR: u8 = 9;
pub const TUBE_PARAM_DAMPING: u8 = 10;
pub const TUBE_PARAM_RESONANCE: u8 = 11;
pub const TUBE_PARAM_MIX: u8 = 12;
pub const TUBE_PARAM_TRIM: u8 = 13;
pub const TUBE_PARAM_COUNT: usize = 14;
/// Subharmonic synth parameter ids for `set_subharm_param` (host-side; each
/// field has its own request).
pub const SUBHARM_PARAM_ENABLED: u8 = 0;
pub const SUBHARM_PARAM_MASK: u8 = 1;
pub const SUBHARM_PARAM_LOW: u8 = 2;
pub const SUBHARM_PARAM_HIGH: u8 = 3;
pub const SUBHARM_PARAM_TOP: u8 = 4;
pub const SUBHARM_PARAM_BOOST: u8 = 5;
pub const SUBHARM_PARAM_SELECT: u8 = 6;
pub const SUBHARM_PARAM_DEPTH: u8 = 7;
pub const SUBHARM_PARAM_HOLD: u8 = 8;
pub const SUBHARM_PARAM_CEILING: u8 = 9;
pub const SUBHARM_PARAM_LINK: u8 = 10;
pub const SUBHARM_PARAM_COUNT: usize = 11;
pub const REQ_SET_EQ_PARAM: u8 = 0x42;
pub const REQ_GET_EQ_PARAM: u8 = 0x43;
/// Legacy: sets every input channel's preamp to the same value.
pub const REQ_SET_PREAMP: u8 = 0x44;
pub const REQ_GET_PREAMP: u8 = 0x45;
pub const REQ_SET_BYPASS: u8 = 0x46;
pub const REQ_GET_BYPASS: u8 = 0x47;
pub const REQ_SET_DELAY: u8 = 0x48;
pub const REQ_GET_DELAY: u8 = 0x49;
pub const REQ_UPMIX_SET_CONFIG: u8 = 0x4A;
pub const REQ_UPMIX_GET_CONFIG: u8 = 0x4B;
pub const REQ_UPMIX_SET_PARAM: u8 = 0x4C;
pub const REQ_UPMIX_GET_PARAM: u8 = 0x4D;
pub const REQ_UPMIX_GET_STATUS: u8 = 0x4E;
/// Upmixer parameter ids (the firmware's own; wValue of REQ_UPMIX_SET_PARAM).
/// Centre mode: 0 passive, 1 adaptive, 2 off. Surround: 0 off, 1 passive, 2 adaptive.
pub const UPMIX_PARAM_ENABLED: u8 = 0;
pub const UPMIX_PARAM_CENTER_MODE: u8 = 1;
pub const UPMIX_PARAM_SURROUND_MODE: u8 = 2;
pub const UPMIX_PARAM_STRENGTH: u8 = 3;
pub const UPMIX_PARAM_WIDTH: u8 = 4;
pub const UPMIX_PARAM_THRESHOLD: u8 = 5;
pub const UPMIX_PARAM_ATTACK: u8 = 6;
pub const UPMIX_PARAM_RELEASE: u8 = 7;
pub const UPMIX_PARAM_DETECTOR_HPF: u8 = 8;
pub const UPMIX_PARAM_SURROUND_DELAY: u8 = 9;
pub const UPMIX_PARAM_SURROUND_HPF: u8 = 10;
pub const UPMIX_PARAM_SURROUND_LPF: u8 = 11;
pub const UPMIX_PARAM_DECORRELATION: u8 = 12;
pub const UPMIX_PARAM_PRESENCE: u8 = 13;
pub const UPMIX_PARAM_COUNT: usize = 14;
pub const REQ_GET_STATUS: u8 = 0x50;
pub const REQ_SAVE_PARAMS: u8 = 0x51;
/// Persist the live output (IO) config in independent mode. Was the
/// deprecated REQ_LOAD_PARAMS, which is gone.
pub const REQ_SAVE_OUTPUT_CONFIG: u8 = 0x52;
pub const REQ_FACTORY_RESET: u8 = 0x53;
pub const REQ_SET_CHANNEL_GAIN: u8 = 0x54;
pub const REQ_GET_CHANNEL_GAIN: u8 = 0x55;
pub const REQ_SET_CHANNEL_MUTE: u8 = 0x56;
pub const REQ_GET_CHANNEL_MUTE: u8 = 0x57;
pub const REQ_SET_LOUDNESS: u8 = 0x58;
pub const REQ_GET_LOUDNESS: u8 = 0x59;
pub const REQ_SET_LOUDNESS_REF: u8 = 0x5A;
pub const REQ_GET_LOUDNESS_REF: u8 = 0x5B;
pub const REQ_SET_LOUDNESS_INTENSITY: u8 = 0x5C;
pub const REQ_GET_LOUDNESS_INTENSITY: u8 = 0x5D;
pub const REQ_SET_LOUDNESS_MASK: u8 = 0xFA;
pub const REQ_GET_LOUDNESS_MASK: u8 = 0xFB;
pub const REQ_SET_CROSSFEED: u8 = 0x5E;
pub const REQ_GET_CROSSFEED: u8 = 0x5F;
pub const REQ_SET_CROSSFEED_PRESET: u8 = 0x60;
pub const REQ_GET_CROSSFEED_PRESET: u8 = 0x61;
pub const REQ_SET_CROSSFEED_FREQ: u8 = 0x62;
pub const REQ_GET_CROSSFEED_FREQ: u8 = 0x63;
pub const REQ_SET_CROSSFEED_FEED: u8 = 0x64;
pub const REQ_GET_CROSSFEED_FEED: u8 = 0x65;
pub const REQ_SET_CROSSFEED_ITD: u8 = 0x66;
pub const REQ_GET_CROSSFEED_ITD: u8 = 0x67;
pub const REQ_SET_CROSSFEED_OUTPUTS: u8 = 0xFC;
pub const REQ_GET_CROSSFEED_OUTPUTS: u8 = 0xFD;
pub const REQ_SET_ADAT_INPUT_ENABLE: u8 = 0x68;
pub const REQ_GET_ADAT_INPUT_ENABLE: u8 = 0x69;
pub const REQ_SET_ADAT_INPUT_PIN: u8 = 0x6A;
pub const REQ_GET_ADAT_INPUT_PIN: u8 = 0x6B;
pub const REQ_SET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6C;
pub const REQ_GET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6D;
pub const REQ_GET_ADAT_INPUT_STATUS: u8 = 0x6E;
pub const REQ_SET_MATRIX_ROUTE: u8 = 0x70;
pub const REQ_GET_MATRIX_ROUTE: u8 = 0x71;
pub const REQ_SET_OUTPUT_ENABLE: u8 = 0x72;
pub const REQ_GET_OUTPUT_ENABLE: u8 = 0x73;
pub const REQ_SET_OUTPUT_GAIN: u8 = 0x74;
pub const REQ_GET_OUTPUT_GAIN: u8 = 0x75;
pub const REQ_SET_OUTPUT_MUTE: u8 = 0x76;
pub const REQ_GET_OUTPUT_MUTE: u8 = 0x77;
pub const REQ_SET_OUTPUT_DELAY: u8 = 0x78;
pub const REQ_GET_OUTPUT_DELAY: u8 = 0x79;
pub const REQ_GET_CORE1_MODE: u8 = 0x7A;
pub const REQ_GET_CORE1_CONFLICT: u8 = 0x7B;
pub const REQ_SET_OUTPUT_PIN: u8 = 0x7C;
pub const REQ_GET_OUTPUT_PIN: u8 = 0x7D;
pub const REQ_GET_SERIAL: u8 = 0x7E;
pub const REQ_GET_PLATFORM: u8 = 0x7F;
pub const REQ_GET_BUILD_INFO: u8 = 0x80;
pub const REQ_LIMITER: u8 = 0x81;
/// REQ_LIMITER wValue low byte: parameter index (high byte = output).
pub const LIMITER_PARAM_ENABLED: u8 = 0;
pub const LIMITER_PARAM_THRESHOLD_DB: u8 = 1;
pub const LIMITER_PARAM_RELEASE_MS: u8 = 2;
pub const LIMITER_PARAM_LINK_GROUP: u8 = 3;
/// GET: per-output gain reduction, u16 in 0.01 dB.
pub const LIMITER_GET_METER: u8 = 0x80;
/// GET: engaged, lookahead, block, num_outputs.
pub const LIMITER_GET_STATUS: u8 = 0x81;
/// SET output byte: apply to every output.
pub const LIMITER_ALL_OUTPUTS: u8 = 0xFF;
pub const REQ_CLEAR_CLIPS: u8 = 0x83;
pub const REQ_SET_CS_BINDING: u8 = 0x84;
pub const REQ_GET_CS_BINDING: u8 = 0x85;
pub const REQ_GET_CS_CAPS: u8 = 0x86;
pub const REQ_GET_CS_STATUS: u8 = 0x87;
pub const REQ_SET_CS_NAME: u8 = 0x8B;
pub const REQ_GET_CS_NAME: u8 = 0x8C;
pub const REQ_SET_CS_IR_CMD: u8 = 0x8D;
pub const REQ_GET_CS_IR_CMD: u8 = 0x8E;
pub const REQ_CS_IR_LEARN: u8 = 0x8F;
pub const REQ_CS_SAVE: u8 = 0x9D;
pub const REQ_CS_REVERT: u8 = 0x9E;
pub const REQ_SET_I2S_CLOCK_MODE: u8 = 0x88;
pub const REQ_GET_I2S_CLOCK_MODE: u8 = 0x89;
pub const REQ_GET_I2S_SLAVE_STATUS: u8 = 0x8A;
pub const REQ_SET_I2S_CLOCK_PIN_MODE: u8 = 0xFE;
pub const REQ_GET_I2S_CLOCK_PIN_MODE: u8 = 0xFF;
pub const REQ_PRESET_SAVE: u8 = 0x90;
pub const REQ_PRESET_LOAD: u8 = 0x91;
pub const REQ_PRESET_DELETE: u8 = 0x92;
pub const REQ_PRESET_GET_NAME: u8 = 0x93;
pub const REQ_PRESET_SET_NAME: u8 = 0x94;
pub const REQ_PRESET_GET_DIR: u8 = 0x95;
pub const REQ_PRESET_SET_STARTUP: u8 = 0x96;
pub const REQ_PRESET_GET_STARTUP: u8 = 0x97;
/// Replaced the old include-pins flag (same 0/1 meaning).
pub const REQ_SET_OUTPUT_CONFIG_MODE: u8 = 0x98;
pub const REQ_GET_OUTPUT_CONFIG_MODE: u8 = 0x99;
pub const REQ_PRESET_GET_ACTIVE: u8 = 0x9A;
pub const REQ_SET_CHANNEL_NAME: u8 = 0x9B;
pub const REQ_GET_CHANNEL_NAME: u8 = 0x9C;
pub const REQ_GET_ALL_PARAMS: u8 = 0xA0;
pub const REQ_SET_ALL_PARAMS: u8 = 0xA1;
pub const REQ_GET_ALL_PARAMS_CHUNK: u8 = 0xA2;
pub const REQ_SET_ALL_PARAMS_CHUNK: u8 = 0xA3;
pub const REQ_SIGGEN_SET_CONFIG: u8 = 0xA4;
pub const REQ_SIGGEN_GET_CONFIG: u8 = 0xA5;
pub const REQ_SIGGEN_CONTROL: u8 = 0xA6;
pub const REQ_SIGGEN_GET_STATUS: u8 = 0xA7;
pub const REQ_SIGGEN_GET_CAPS: u8 = 0xA8;
pub const REQ_GET_BUFFER_STATS: u8 = 0xB0;
pub const REQ_RESET_BUFFER_STATS: u8 = 0xB1;
pub const REQ_GET_USB_ERROR_STATS: u8 = 0xB2;
pub const REQ_RESET_USB_ERROR_STATS: u8 = 0xB3;
pub const REQ_SET_LEVELLER_ENABLE: u8 = 0xB4;
pub const REQ_GET_LEVELLER_ENABLE: u8 = 0xB5;
pub const REQ_SET_LEVELLER_AMOUNT: u8 = 0xB6;
pub const REQ_GET_LEVELLER_AMOUNT: u8 = 0xB7;
pub const REQ_SET_LEVELLER_SPEED: u8 = 0xB8;
pub const REQ_GET_LEVELLER_SPEED: u8 = 0xB9;
pub const REQ_SET_LEVELLER_MAX_GAIN: u8 = 0xBA;
pub const REQ_GET_LEVELLER_MAX_GAIN: u8 = 0xBB;
pub const REQ_SET_LEVELLER_LOOKAHEAD: u8 = 0xBC;
pub const REQ_GET_LEVELLER_LOOKAHEAD: u8 = 0xBD;
pub const REQ_SET_LEVELLER_GATE: u8 = 0xBE;
pub const REQ_GET_LEVELLER_GATE: u8 = 0xBF;
pub const REQ_SET_LEVELLER_MASKS: u8 = 0xDE;
pub const REQ_GET_LEVELLER_MASKS: u8 = 0xDF;
pub const REQ_SET_OUTPUT_TYPE: u8 = 0xC0;
pub const REQ_GET_OUTPUT_TYPE: u8 = 0xC1;
pub const REQ_SET_I2S_BCK_PIN: u8 = 0xC2;
pub const REQ_GET_I2S_BCK_PIN: u8 = 0xC3;
pub const REQ_SET_MCK_ENABLE: u8 = 0xC4;
pub const REQ_GET_MCK_ENABLE: u8 = 0xC5;
pub const REQ_SET_MCK_PIN: u8 = 0xC6;
pub const REQ_GET_MCK_PIN: u8 = 0xC7;
pub const REQ_SET_MCK_MULTIPLIER: u8 = 0xC8;
pub const REQ_GET_MCK_MULTIPLIER: u8 = 0xC9;
pub const REQ_SET_ADAT_ENABLE: u8 = 0xCA;
pub const REQ_GET_ADAT_ENABLE: u8 = 0xCB;
pub const REQ_SET_ADAT_PIN: u8 = 0xCC;
pub const REQ_GET_ADAT_PIN: u8 = 0xCD;
pub const REQ_GET_ADAT_STATUS: u8 = 0xCE;
pub const REQ_SET_PREAMP_CH: u8 = 0xD0;
pub const REQ_GET_PREAMP_CH: u8 = 0xD1;
pub const REQ_SET_MASTER_VOLUME: u8 = 0xD2;
pub const REQ_GET_MASTER_VOLUME: u8 = 0xD3;
pub const REQ_SET_MASTER_VOLUME_MODE: u8 = 0xD4;
pub const REQ_GET_MASTER_VOLUME_MODE: u8 = 0xD5;
pub const REQ_SAVE_MASTER_VOLUME: u8 = 0xD6;
pub const REQ_GET_SAVED_MASTER_VOLUME: u8 = 0xD7;
pub const REQ_SET_BAND_BYPASS: u8 = 0xD8;
pub const REQ_GET_BAND_BYPASS: u8 = 0xD9;
pub const REQ_SET_USER_VOLUME: u8 = 0xDA;
pub const REQ_GET_USER_VOLUME: u8 = 0xDB;
pub const REQ_SET_USER_MUTE: u8 = 0xDC;
pub const REQ_GET_USER_MUTE: u8 = 0xDD;
pub const REQ_SET_INPUT_SOURCE: u8 = 0xE0;
pub const REQ_GET_INPUT_SOURCE: u8 = 0xE1;
pub const REQ_GET_SPDIF_RX_STATUS: u8 = 0xE2;
pub const REQ_GET_SPDIF_RX_CH_STATUS: u8 = 0xE3;
pub const REQ_SET_SPDIF_RX_PIN: u8 = 0xE4;
pub const REQ_GET_SPDIF_RX_PIN: u8 = 0xE5;
pub const REQ_SET_SPDIF_INPUT_ENABLE: u8 = 0xE9;
pub const REQ_GET_SPDIF_INPUT_CONFIG: u8 = 0xEF;
pub const REQ_SET_INPUT_RATE: u8 = 0xED;
pub const REQ_GET_INPUT_RATE: u8 = 0xEE;
pub const REQ_SET_I2S_RX_PIN: u8 = 0xF1;
pub const REQ_GET_I2S_RX_PIN: u8 = 0xF2;
pub const REQ_SET_I2S_INPUT_CHANNELS: u8 = 0xF3;
pub const REQ_GET_I2S_INPUT_CHANNELS: u8 = 0xF4;
pub const REQ_SET_LG_SOUND_SYNC_ENABLE: u8 = 0xE6;
pub const REQ_GET_LG_SOUND_SYNC_ENABLE: u8 = 0xE7;
pub const REQ_GET_LG_SOUND_SYNC_STATUS: u8 = 0xE8;
pub const REQ_SET_DAC_HW_MUTE_CONFIG: u8 = 0xEA;
pub const REQ_GET_DAC_HW_MUTE_CONFIG: u8 = 0xEB;
pub const REQ_TEST_DAC_HW_MUTE: u8 = 0xEC;
pub const REQ_ENTER_BOOTLOADER: u8 = 0xF0;
pub const REQ_SET_UART_CONFIG: u8 = 0xF5;
pub const REQ_GET_UART_CONFIG: u8 = 0xF6;
pub const REQ_SET_I2C_CONFIG: u8 = 0xF7;
pub const REQ_GET_I2C_CONFIG: u8 = 0xF8;
pub const REQ_GET_CTRL_IFACE_STATUS: u8 = 0xF9;
pub const PIN_CONFIG_SUCCESS: u8 = 0x00;
pub const PIN_CONFIG_INVALID_PIN: u8 = 0x01;
pub const PIN_CONFIG_PIN_IN_USE: u8 = 0x02;
pub const PIN_CONFIG_INVALID_OUTPUT: u8 = 0x03;
pub const PIN_CONFIG_OUTPUT_ACTIVE: u8 = 0x04;
pub const PIN_CONFIG_INVALID_PARAM: u8 = 0x05;
/// Sent as the pin byte of any single-pin SET to restore the platform default.
pub const PIN_RESET_TO_DEFAULT: u8 = 0xFF;
pub const PRESET_OK: u8 = 0x00;
pub const PRESET_ERR_INVALID_SLOT: u8 = 0x01;
pub const PRESET_ERR_SLOT_EMPTY: u8 = 0x02;
pub const PRESET_ERR_CRC: u8 = 0x03;
pub const PRESET_ERR_FLASH_WRITE: u8 = 0x04;
pub const FLASH_OK: u8 = 0;
pub const FLASH_ERR_WRITE: u8 = 1;
pub const FLASH_ERR_NO_DATA: u8 = 2;
pub const FLASH_ERR_CRC: u8 = 3;
/// wIndex for EQ, preamp, bypass, delay, status, loudness, crossfeed, volume, core1.
pub const WINDEX_GLOBAL: u16 = 0;
/// wIndex for matrix/output/pin/preset/channel-name/bulk commands.
pub const WINDEX_OUTPUT: u16 = 2;
/// Wire format this core reads and writes.
pub const WIRE_FORMAT_VERSION: u8 = 32;
/// Oldest wire format with the unified channel model. Anything older is
/// incompatible (V16 broke compatibility with no migration).
pub const MIN_WIRE_FORMAT_VERSION: u8 = 16;
/// sizeof(WireBulkParams) at V32.
pub const BULK_PARAMS_SIZE: usize = 6136;
/// Chunk size for REQ_GET/SET_ALL_PARAMS_CHUNK. Kept well under the 4 KB
/// control-transfer limit (Linux usbfs caps a control transfer at one page).
pub const BULK_CHUNK_SIZE: usize = 2048;
pub const OFF_HEADER: usize = 0;
pub const OFF_GLOBAL: usize = 16;
pub const OFF_CROSSFEED: usize = 32;
pub const OFF_LEGACY: usize = 48;
pub const OFF_DELAYS: usize = 64;
pub const OFF_CROSSPOINTS: usize = 132;
pub const OFF_OUTPUTS: usize = 708;
pub const OFF_PINS: usize = 816;
pub const OFF_EQ: usize = 824;
pub const OFF_CHANNEL_NAMES: usize = 4088;
pub const OFF_I2S: usize = 4632;
pub const OFF_LEVELLER: usize = 4648;
pub const OFF_PREAMP: usize = 4668;
pub const OFF_MASTER_VOLUME: usize = 4700;
pub const OFF_INPUT_CONFIG: usize = 4716;
pub const OFF_LG_SOUND_SYNC: usize = 4732;
pub const OFF_USER_VOLUME: usize = 4748;
pub const OFF_DAC_HW_MUTE: usize = 4764;
pub const OFF_CROSSOVER: usize = 4780;
pub const OFF_ADAT: usize = 5868;
pub const OFF_PSYBASS: usize = 5876;
pub const OFF_UPMIX: usize = 5900;
pub const OFF_SUBHARM: usize = 5944;
pub const OFF_TUBE: usize = 5980;
pub const OFF_LIMITER: usize = 6028;
/// `fw_beta` for a beta that can't say which one it is: 1.1.6 betas 1 and 2
/// predate the ordinal and answer as plain 1.1.6. Sorts below every beta.
pub const FW_BETA_EARLY: u8 = 0xFF;

// ── rta.rs ─────────────────────────────────────────────────────────

pub const RTA_VERSION: u8 = 3;
pub const RTA_MAX_BANDS: usize = 37;
/// Channels at one tap the snapshot holds (the mask is 16 bits).
pub const RTA_MAX_CHANNELS: usize = 16;
pub const RTA_MAX_BINS: usize = 512;
pub const RTA_TAP_INPUT: u8 = 0;
pub const RTA_TAP_OUTPUT: u8 = 1;
/// Level byte of the band and bin frames at the floor: -121.5 dBFS.
pub const RTA_FLOOR_DB: f32 = -121.5;

// ── siggen.rs ──────────────────────────────────────────────────────

pub const SIGGEN_VERSION: u8 = 1;
pub const SIGGEN_TYPE_COUNT: usize = 15;
pub const SIGGEN_FLAG_RAW: u8 = 0x01;
pub const SIGGEN_FLAG_DECORR: u8 = 0x02;
pub const SIGGEN_FLAG_WALK: u8 = 0x04;
/// CONTROL actions.
pub const SIGGEN_CTL_STOP: u16 = 0;
pub const SIGGEN_CTL_START: u16 = 1;
pub const SIGGEN_CTL_STOP_NOW: u16 = 2;

// ── stats.rs ───────────────────────────────────────────────────────

/// REQ_GET_STATUS sub-queries that return one u32 (cumulative since boot).
pub const STAT_PDM_RING_OVERRUNS: u16 = 3;
pub const STAT_PDM_RING_UNDERRUNS: u16 = 4;
pub const STAT_PDM_DMA_OVERRUNS: u16 = 5;
pub const STAT_PDM_DMA_UNDERRUNS: u16 = 6;
pub const STAT_SPDIF_OVERRUNS: u16 = 7;
pub const STAT_SPDIF_UNDERRUNS: u16 = 8;
pub const STAT_CLOCK_HZ: u16 = 13;
pub const STAT_CORE_MV: u16 = 14;
pub const STAT_SAMPLE_RATE: u16 = 15;
/// Centi-degrees C, signed (an i16 widened to u32).
pub const STAT_TEMPERATURE: u16 = 16;
/// S/PDIF DMA starvations: total, then instances 0-3 (18-21).
pub const STAT_STARVATION_TOTAL: u16 = 17;
pub const STAT_STARVATION_FIRST: u16 = 18;
pub const STAT_USB_RING_OVERRUNS: u16 = 22;

// ── presetfile.rs ──────────────────────────────────────────────────

pub const SCHEMA_VERSION: i64 = 1;
pub const IMPORT_VOLUMES: u32 = 1;
pub const IMPORT_HARDWARE: u32 = 2;

// ── filterfile.rs ──────────────────────────────────────────────────

pub const FORMAT_VERSION: u32 = 2;

// ── cs.rs ──────────────────────────────────────────────────────────

pub const CS_MAX_BINDINGS: usize = 16;
pub const CS_MAX_IR: usize = 16;
pub const CS_MAX_GROUPS: usize = 8;
pub const CS_MAX_MACROS: usize = 8;
pub const CS_MAX_STEPS: usize = 8;
pub const CS_MAX_PAGES: usize = 16;

// ── state.rs ───────────────────────────────────────────────────────

/// Firmware compatibility of the connected device, as judged at connect time.
pub const COMPAT_UNKNOWN: u8 = 0;
/// Wire format and firmware match this core.
pub const COMPAT_OK: u8 = 1;
/// Firmware is too old (pre-V32 wire format, or the old USB vendor ID).
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;
/// Firmware is newer than this core understands.
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

// ── types.rs ───────────────────────────────────────────────────────

pub const FILTER_FLAT: u8 = 0;
pub const FILTER_PEAKING: u8 = 1;
pub const FILTER_LOWSHELF: u8 = 2;
pub const FILTER_HIGHSHELF: u8 = 3;
pub const FILTER_LOWPASS: u8 = 4;
pub const FILTER_HIGHPASS: u8 = 5;
pub const FILTER_NOTCH: u8 = 6;
/// Second-order (RBJ) all-pass.
pub const FILTER_ALLPASS: u8 = 7;
/// First-order all-pass: frequency only.
pub const FILTER_ALLPASS1: u8 = 8;
/// First-order low shelf: frequency + gain, no Q.
pub const FILTER_LOWSHELF1: u8 = 9;
/// First-order high shelf: frequency + gain, no Q.
pub const FILTER_HIGHSHELF1: u8 = 10;
/// Linkwitz Transform: freq = f0, q = Q0, gain carries fp in Hz, qp = target Q.
pub const FILTER_LINKWITZ_TRANSFORM: u8 = 11;
/// First-order low pass (6 dB/oct): frequency only.
pub const FILTER_LOWPASS1: u8 = 12;
/// First-order high pass (6 dB/oct): frequency only.
pub const FILTER_HIGHPASS1: u8 = 13;
/// First crossover type (LR2 LP). Crossover types run 32..=63:
/// LR2/4/6/8 = 32..39, Butterworth 1..8 = 40..55, Bessel 2/4/6/8 = 56..63,
/// each pair LP = even, HP = odd.
pub const FILTER_XOVER_FIRST: u8 = 32;
pub const FILTER_XOVER_LAST: u8 = 63;
/// Default Q (Butterworth), also the Linkwitz Transform target-Q default.
pub const DEFAULT_Q: f32 = 0.707;
/// Maximum channels in the wire format (inputs + outputs, RP2350).
pub const MAX_CHANNELS: usize = 17;
/// Maximum input channels (RP2350).
pub const MAX_INPUTS: usize = 8;
/// Maximum output channels (RP2350).
pub const MAX_OUTPUTS: usize = 9;
/// PEQ bands per channel that the firmware processes.
pub const BANDS_PER_CHANNEL: usize = 10;
/// PEQ band storage per channel in the wire layout.
pub const FIRMWARE_BANDS_PER_CHANNEL: usize = 12;
/// Crossover bands per output channel.
pub const MAX_XOVER_BANDS: usize = 4;
/// Wire band index of crossover band 0 (crossover bands are 20..23).
pub const XOVER_BAND_BASE: u8 = 20;
/// Maximum preset slots.
pub const MAX_PRESETS: usize = 10;
/// Channel / preset name buffer size.
pub const CHANNEL_NAME_LEN: usize = 32;
/// Physical output pin slots (4 S/PDIF + 1 PDM on RP2350).
pub const MAX_PHYSICAL_OUTPUTS: usize = 5;
/// Master volume mute sentinel (true −∞).
pub const MASTER_VOL_MUTE_DB: f32 = -128.0;
