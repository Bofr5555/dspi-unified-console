// Generated, do not edit: regenerate with `cargo run -p xtask -- capi-consts`.
// The values come from dspi-proto (the firmware headers), dspi-core and
// dspi-session; ci/drift.sh fails when this file is stale.

//! The C ABI's constants: each is a `#define` in `dspi_core.h`.

// `1 << 0` spells a flag like its neighbours.
#![allow(clippy::identity_op)]

// ── usb ── USB identity and control transfers

/// `dspi_proto::USB_VID`.
pub const VENDOR_ID: u16 = 0x2E8B;

/// `dspi_proto::USB_PID`.
pub const PRODUCT_ID: u16 = 0xFEAA;

/// `dspi_proto::VENDOR_INTERFACE`.
pub const VENDOR_INTERFACE: u8 = 2;

/// `dspi_proto::generated::usb::VENDOR_EP_IN`.
pub const NOTIFY_ENDPOINT: u8 = 0x83;

/// `dspi_proto::REQ_TYPE_OUT`.
pub const REQ_TYPE_OUT: u8 = 0x41;

/// `dspi_proto::REQ_TYPE_IN`.
pub const REQ_TYPE_IN: u8 = 0xC1;

/// Raspberry Pi's vendor id, which early DSPi firmware enumerated with.
pub const LEGACY_VENDOR_ID: u16 = 0x2E8A;

/// `wIndex` of the old ABI's global requests.
pub const WINDEX_GLOBAL: u16 = 0;

/// `wIndex` of the old ABI's per-output requests.
pub const WINDEX_OUTPUT: u16 = 2;

// ── opcodes ── Vendor request opcodes (bRequest)

/// `dspi_proto::generated::opcodes::REQ_SET_CS_AUX_STATE`.
pub const REQ_SET_CS_AUX_STATE: u8 = 0x04;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_AUX_STATE`.
pub const REQ_GET_CS_AUX_STATE: u8 = 0x05;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_AUX_LEVEL`.
pub const REQ_SET_CS_AUX_LEVEL: u8 = 0x06;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_AUX_LEVEL`.
pub const REQ_GET_CS_AUX_LEVEL: u8 = 0x07;

/// `dspi_proto::generated::opcodes::REQ_RTA_SET_CONFIG`.
pub const REQ_RTA_SET_CONFIG: u8 = 0x08;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_CONFIG`.
pub const REQ_RTA_GET_CONFIG: u8 = 0x09;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_CAPS`.
pub const REQ_RTA_GET_CAPS: u8 = 0x0A;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_BANDS`.
pub const REQ_RTA_GET_BANDS: u8 = 0x0B;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_BINS`.
pub const REQ_RTA_GET_BINS: u8 = 0x0C;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_STATUS`.
pub const REQ_RTA_GET_STATUS: u8 = 0x0D;

/// `dspi_proto::generated::opcodes::REQ_RTA_CONTROL`.
pub const REQ_RTA_CONTROL: u8 = 0x0E;

/// `dspi_proto::generated::opcodes::REQ_RTA_GET_BANDS_ALL`.
pub const REQ_RTA_GET_BANDS_ALL: u8 = 0x0F;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM`.
pub const REQ_SET_SUBHARM: u8 = 0x10;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM`.
pub const REQ_GET_SUBHARM: u8 = 0x11;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_LOW`.
pub const REQ_SET_SUBHARM_LOW: u8 = 0x12;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_LOW`.
pub const REQ_GET_SUBHARM_LOW: u8 = 0x13;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_HIGH`.
pub const REQ_SET_SUBHARM_HIGH: u8 = 0x14;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_HIGH`.
pub const REQ_GET_SUBHARM_HIGH: u8 = 0x15;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_BOOST`.
pub const REQ_SET_SUBHARM_BOOST: u8 = 0x16;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_BOOST`.
pub const REQ_GET_SUBHARM_BOOST: u8 = 0x17;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_MASK`.
pub const REQ_SET_SUBHARM_MASK: u8 = 0x18;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_MASK`.
pub const REQ_GET_SUBHARM_MASK: u8 = 0x19;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_HEADROOM`.
pub const REQ_GET_SUBHARM_HEADROOM: u8 = 0x1A;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_TOP`.
pub const REQ_SET_SUBHARM_TOP: u8 = 0x1B;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_TOP`.
pub const REQ_GET_SUBHARM_TOP: u8 = 0x1C;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_SELECT`.
pub const REQ_SET_SUBHARM_SELECT: u8 = 0x1D;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_SELECT`.
pub const REQ_GET_SUBHARM_SELECT: u8 = 0x1E;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_METER`.
pub const REQ_GET_SUBHARM_METER: u8 = 0x1F;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_GROUP`.
pub const REQ_SET_CS_GROUP: u8 = 0x20;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_GROUP`.
pub const REQ_GET_CS_GROUP: u8 = 0x21;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_MACRO`.
pub const REQ_SET_CS_MACRO: u8 = 0x22;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_MACRO`.
pub const REQ_GET_CS_MACRO: u8 = 0x23;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_MACRO_STEP`.
pub const REQ_SET_CS_MACRO_STEP: u8 = 0x24;

/// `dspi_proto::generated::opcodes::REQ_CS_MACRO_FIRE`.
pub const REQ_CS_MACRO_FIRE: u8 = 0x25;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_EXT_STATUS`.
pub const REQ_GET_CS_EXT_STATUS: u8 = 0x26;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_DISPLAY_CFG`.
pub const REQ_SET_CS_DISPLAY_CFG: u8 = 0x27;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_DISPLAY_CFG`.
pub const REQ_GET_CS_DISPLAY_CFG: u8 = 0x28;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_DISPLAY_PAGE`.
pub const REQ_SET_CS_DISPLAY_PAGE: u8 = 0x29;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_DISPLAY_PAGE`.
pub const REQ_GET_CS_DISPLAY_PAGE: u8 = 0x2A;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_DISPLAY_STATUS`.
pub const REQ_GET_CS_DISPLAY_STATUS: u8 = 0x2B;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_SOLO`.
pub const REQ_SET_SUBHARM_SOLO: u8 = 0x2C;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_SOLO`.
pub const REQ_GET_SUBHARM_SOLO: u8 = 0x2D;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_LINK`.
pub const REQ_SET_SUBHARM_LINK: u8 = 0x2E;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_LINK`.
pub const REQ_GET_SUBHARM_LINK: u8 = 0x2F;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS`.
pub const REQ_SET_PSYBASS: u8 = 0x30;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS`.
pub const REQ_GET_PSYBASS: u8 = 0x31;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_CUTOFF`.
pub const REQ_SET_PSYBASS_CUTOFF: u8 = 0x32;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_CUTOFF`.
pub const REQ_GET_PSYBASS_CUTOFF: u8 = 0x33;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_HARMONICS`.
pub const REQ_SET_PSYBASS_HARMONICS: u8 = 0x34;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_HARMONICS`.
pub const REQ_GET_PSYBASS_HARMONICS: u8 = 0x35;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_DRIVE`.
pub const REQ_SET_PSYBASS_DRIVE: u8 = 0x36;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_DRIVE`.
pub const REQ_GET_PSYBASS_DRIVE: u8 = 0x37;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_CHARACTER`.
pub const REQ_SET_PSYBASS_CHARACTER: u8 = 0x38;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_CHARACTER`.
pub const REQ_GET_PSYBASS_CHARACTER: u8 = 0x39;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_ORIGINAL`.
pub const REQ_SET_PSYBASS_ORIGINAL: u8 = 0x3A;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_ORIGINAL`.
pub const REQ_GET_PSYBASS_ORIGINAL: u8 = 0x3B;

/// `dspi_proto::generated::opcodes::REQ_SET_PSYBASS_MASK`.
pub const REQ_SET_PSYBASS_MASK: u8 = 0x3C;

/// `dspi_proto::generated::opcodes::REQ_GET_PSYBASS_MASK`.
pub const REQ_GET_PSYBASS_MASK: u8 = 0x3D;

/// `dspi_proto::generated::opcodes::REQ_SET_TUBE_PARAM`.
pub const REQ_SET_TUBE_PARAM: u8 = 0x3E;

/// `dspi_proto::generated::opcodes::REQ_GET_TUBE_PARAM`.
pub const REQ_GET_TUBE_PARAM: u8 = 0x3F;

/// `dspi_proto::generated::opcodes::REQ_SET_EQ_PARAM`.
pub const REQ_SET_EQ_PARAM: u8 = 0x42;

/// `dspi_proto::generated::opcodes::REQ_GET_EQ_PARAM`.
pub const REQ_GET_EQ_PARAM: u8 = 0x43;

/// `dspi_proto::generated::opcodes::REQ_SET_PREAMP`.
pub const REQ_SET_PREAMP: u8 = 0x44;

/// `dspi_proto::generated::opcodes::REQ_GET_PREAMP`.
pub const REQ_GET_PREAMP: u8 = 0x45;

/// `dspi_proto::generated::opcodes::REQ_SET_BYPASS`.
pub const REQ_SET_BYPASS: u8 = 0x46;

/// `dspi_proto::generated::opcodes::REQ_GET_BYPASS`.
pub const REQ_GET_BYPASS: u8 = 0x47;

/// `dspi_proto::generated::opcodes::REQ_SET_DELAY`.
pub const REQ_SET_DELAY: u8 = 0x48;

/// `dspi_proto::generated::opcodes::REQ_GET_DELAY`.
pub const REQ_GET_DELAY: u8 = 0x49;

/// `dspi_proto::generated::opcodes::REQ_UPMIX_SET_CONFIG`.
pub const REQ_UPMIX_SET_CONFIG: u8 = 0x4A;

/// `dspi_proto::generated::opcodes::REQ_UPMIX_GET_CONFIG`.
pub const REQ_UPMIX_GET_CONFIG: u8 = 0x4B;

/// `dspi_proto::generated::opcodes::REQ_UPMIX_SET_PARAM`.
pub const REQ_UPMIX_SET_PARAM: u8 = 0x4C;

/// `dspi_proto::generated::opcodes::REQ_UPMIX_GET_PARAM`.
pub const REQ_UPMIX_GET_PARAM: u8 = 0x4D;

/// `dspi_proto::generated::opcodes::REQ_UPMIX_GET_STATUS`.
pub const REQ_UPMIX_GET_STATUS: u8 = 0x4E;

/// `dspi_proto::generated::opcodes::REQ_GET_STATUS`.
pub const REQ_GET_STATUS: u8 = 0x50;

/// `dspi_proto::generated::opcodes::REQ_SAVE_PARAMS`.
pub const REQ_SAVE_PARAMS: u8 = 0x51;

/// `dspi_proto::generated::opcodes::REQ_SAVE_OUTPUT_CONFIG`.
pub const REQ_SAVE_OUTPUT_CONFIG: u8 = 0x52;

/// `dspi_proto::generated::opcodes::REQ_FACTORY_RESET`.
pub const REQ_FACTORY_RESET: u8 = 0x53;

/// Older firmware's opcode, kept for the ABI; current firmware uses this value for a name request.
pub const REQ_SET_CHANNEL_GAIN: u8 = 0x54;

/// Older firmware's opcode, kept for the ABI; current firmware uses this value for a name request.
pub const REQ_GET_CHANNEL_GAIN: u8 = 0x55;

/// Older firmware's opcode, kept for the ABI; current firmware uses this value for a name request.
pub const REQ_SET_CHANNEL_MUTE: u8 = 0x56;

/// Older firmware's opcode, kept for the ABI; current firmware uses this value for a name request.
pub const REQ_GET_CHANNEL_MUTE: u8 = 0x57;

/// `dspi_proto::generated::opcodes::REQ_SET_LOUDNESS`.
pub const REQ_SET_LOUDNESS: u8 = 0x58;

/// `dspi_proto::generated::opcodes::REQ_GET_LOUDNESS`.
pub const REQ_GET_LOUDNESS: u8 = 0x59;

/// `dspi_proto::generated::opcodes::REQ_SET_LOUDNESS_REF`.
pub const REQ_SET_LOUDNESS_REF: u8 = 0x5A;

/// `dspi_proto::generated::opcodes::REQ_GET_LOUDNESS_REF`.
pub const REQ_GET_LOUDNESS_REF: u8 = 0x5B;

/// `dspi_proto::generated::opcodes::REQ_SET_LOUDNESS_INTENSITY`.
pub const REQ_SET_LOUDNESS_INTENSITY: u8 = 0x5C;

/// `dspi_proto::generated::opcodes::REQ_GET_LOUDNESS_INTENSITY`.
pub const REQ_GET_LOUDNESS_INTENSITY: u8 = 0x5D;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED`.
pub const REQ_SET_CROSSFEED: u8 = 0x5E;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED`.
pub const REQ_GET_CROSSFEED: u8 = 0x5F;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED_PRESET`.
pub const REQ_SET_CROSSFEED_PRESET: u8 = 0x60;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED_PRESET`.
pub const REQ_GET_CROSSFEED_PRESET: u8 = 0x61;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED_FREQ`.
pub const REQ_SET_CROSSFEED_FREQ: u8 = 0x62;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED_FREQ`.
pub const REQ_GET_CROSSFEED_FREQ: u8 = 0x63;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED_FEED`.
pub const REQ_SET_CROSSFEED_FEED: u8 = 0x64;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED_FEED`.
pub const REQ_GET_CROSSFEED_FEED: u8 = 0x65;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED_ITD`.
pub const REQ_SET_CROSSFEED_ITD: u8 = 0x66;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED_ITD`.
pub const REQ_GET_CROSSFEED_ITD: u8 = 0x67;

/// `dspi_proto::generated::opcodes::REQ_SET_ADAT_INPUT_ENABLE`.
pub const REQ_SET_ADAT_INPUT_ENABLE: u8 = 0x68;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_INPUT_ENABLE`.
pub const REQ_GET_ADAT_INPUT_ENABLE: u8 = 0x69;

/// `dspi_proto::generated::opcodes::REQ_SET_ADAT_INPUT_PIN`.
pub const REQ_SET_ADAT_INPUT_PIN: u8 = 0x6A;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_INPUT_PIN`.
pub const REQ_GET_ADAT_INPUT_PIN: u8 = 0x6B;

/// `dspi_proto::generated::opcodes::REQ_SET_ADAT_INPUT_CLOCK_MODE`.
pub const REQ_SET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6C;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_INPUT_CLOCK_MODE`.
pub const REQ_GET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6D;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_INPUT_STATUS`.
pub const REQ_GET_ADAT_INPUT_STATUS: u8 = 0x6E;

/// `dspi_proto::generated::opcodes::REQ_SET_MATRIX_ROUTE`.
pub const REQ_SET_MATRIX_ROUTE: u8 = 0x70;

/// `dspi_proto::generated::opcodes::REQ_GET_MATRIX_ROUTE`.
pub const REQ_GET_MATRIX_ROUTE: u8 = 0x71;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_ENABLE`.
pub const REQ_SET_OUTPUT_ENABLE: u8 = 0x72;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_ENABLE`.
pub const REQ_GET_OUTPUT_ENABLE: u8 = 0x73;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_GAIN`.
pub const REQ_SET_OUTPUT_GAIN: u8 = 0x74;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_GAIN`.
pub const REQ_GET_OUTPUT_GAIN: u8 = 0x75;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_MUTE`.
pub const REQ_SET_OUTPUT_MUTE: u8 = 0x76;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_MUTE`.
pub const REQ_GET_OUTPUT_MUTE: u8 = 0x77;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_DELAY`.
pub const REQ_SET_OUTPUT_DELAY: u8 = 0x78;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_DELAY`.
pub const REQ_GET_OUTPUT_DELAY: u8 = 0x79;

/// `dspi_proto::generated::opcodes::REQ_GET_CORE1_MODE`.
pub const REQ_GET_CORE1_MODE: u8 = 0x7A;

/// `dspi_proto::generated::opcodes::REQ_GET_CORE1_CONFLICT`.
pub const REQ_GET_CORE1_CONFLICT: u8 = 0x7B;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_PIN`.
pub const REQ_SET_OUTPUT_PIN: u8 = 0x7C;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_PIN`.
pub const REQ_GET_OUTPUT_PIN: u8 = 0x7D;

/// `dspi_proto::generated::opcodes::REQ_GET_SERIAL`.
pub const REQ_GET_SERIAL: u8 = 0x7E;

/// `dspi_proto::generated::opcodes::REQ_GET_PLATFORM`.
pub const REQ_GET_PLATFORM: u8 = 0x7F;

/// `dspi_proto::generated::opcodes::REQ_GET_BUILD_INFO`.
pub const REQ_GET_BUILD_INFO: u8 = 0x80;

/// `dspi_proto::generated::opcodes::REQ_LIMITER`.
pub const REQ_LIMITER: u8 = 0x81;

/// `dspi_proto::generated::opcodes::REQ_CLEAR_CLIPS`.
pub const REQ_CLEAR_CLIPS: u8 = 0x83;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_BINDING`.
pub const REQ_SET_CS_BINDING: u8 = 0x84;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_BINDING`.
pub const REQ_GET_CS_BINDING: u8 = 0x85;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_CAPS`.
pub const REQ_GET_CS_CAPS: u8 = 0x86;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_STATUS`.
pub const REQ_GET_CS_STATUS: u8 = 0x87;

/// `dspi_proto::generated::opcodes::REQ_SET_I2S_CLOCK_MODE`.
pub const REQ_SET_I2S_CLOCK_MODE: u8 = 0x88;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_CLOCK_MODE`.
pub const REQ_GET_I2S_CLOCK_MODE: u8 = 0x89;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_SLAVE_STATUS`.
pub const REQ_GET_I2S_SLAVE_STATUS: u8 = 0x8A;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_NAME`.
pub const REQ_SET_CS_NAME: u8 = 0x8B;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_NAME`.
pub const REQ_GET_CS_NAME: u8 = 0x8C;

/// `dspi_proto::generated::opcodes::REQ_SET_CS_IR_CMD`.
pub const REQ_SET_CS_IR_CMD: u8 = 0x8D;

/// `dspi_proto::generated::opcodes::REQ_GET_CS_IR_CMD`.
pub const REQ_GET_CS_IR_CMD: u8 = 0x8E;

/// `dspi_proto::generated::opcodes::REQ_CS_IR_LEARN`.
pub const REQ_CS_IR_LEARN: u8 = 0x8F;

/// `dspi_proto::generated::opcodes::REQ_PRESET_SAVE`.
pub const REQ_PRESET_SAVE: u8 = 0x90;

/// `dspi_proto::generated::opcodes::REQ_PRESET_LOAD`.
pub const REQ_PRESET_LOAD: u8 = 0x91;

/// `dspi_proto::generated::opcodes::REQ_PRESET_DELETE`.
pub const REQ_PRESET_DELETE: u8 = 0x92;

/// `dspi_proto::generated::opcodes::REQ_PRESET_GET_NAME`.
pub const REQ_PRESET_GET_NAME: u8 = 0x93;

/// `dspi_proto::generated::opcodes::REQ_PRESET_SET_NAME`.
pub const REQ_PRESET_SET_NAME: u8 = 0x94;

/// `dspi_proto::generated::opcodes::REQ_PRESET_GET_DIR`.
pub const REQ_PRESET_GET_DIR: u8 = 0x95;

/// `dspi_proto::generated::opcodes::REQ_PRESET_SET_STARTUP`.
pub const REQ_PRESET_SET_STARTUP: u8 = 0x96;

/// `dspi_proto::generated::opcodes::REQ_PRESET_GET_STARTUP`.
pub const REQ_PRESET_GET_STARTUP: u8 = 0x97;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_CONFIG_MODE`.
pub const REQ_SET_OUTPUT_CONFIG_MODE: u8 = 0x98;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_CONFIG_MODE`.
pub const REQ_GET_OUTPUT_CONFIG_MODE: u8 = 0x99;

/// `dspi_proto::generated::opcodes::REQ_PRESET_GET_ACTIVE`.
pub const REQ_PRESET_GET_ACTIVE: u8 = 0x9A;

/// `dspi_proto::generated::opcodes::REQ_SET_CHANNEL_NAME`.
pub const REQ_SET_CHANNEL_NAME: u8 = 0x9B;

/// `dspi_proto::generated::opcodes::REQ_GET_CHANNEL_NAME`.
pub const REQ_GET_CHANNEL_NAME: u8 = 0x9C;

/// `dspi_proto::generated::opcodes::REQ_CS_SAVE`.
pub const REQ_CS_SAVE: u8 = 0x9D;

/// `dspi_proto::generated::opcodes::REQ_CS_REVERT`.
pub const REQ_CS_REVERT: u8 = 0x9E;

/// `dspi_proto::generated::opcodes::REQ_GET_ALL_PARAMS`.
pub const REQ_GET_ALL_PARAMS: u8 = 0xA0;

/// `dspi_proto::generated::opcodes::REQ_SET_ALL_PARAMS`.
pub const REQ_SET_ALL_PARAMS: u8 = 0xA1;

/// `dspi_proto::generated::opcodes::REQ_GET_ALL_PARAMS_CHUNK`.
pub const REQ_GET_ALL_PARAMS_CHUNK: u8 = 0xA2;

/// `dspi_proto::generated::opcodes::REQ_SET_ALL_PARAMS_CHUNK`.
pub const REQ_SET_ALL_PARAMS_CHUNK: u8 = 0xA3;

/// `dspi_proto::generated::opcodes::REQ_SIGGEN_SET_CONFIG`.
pub const REQ_SIGGEN_SET_CONFIG: u8 = 0xA4;

/// `dspi_proto::generated::opcodes::REQ_SIGGEN_GET_CONFIG`.
pub const REQ_SIGGEN_GET_CONFIG: u8 = 0xA5;

/// `dspi_proto::generated::opcodes::REQ_SIGGEN_CONTROL`.
pub const REQ_SIGGEN_CONTROL: u8 = 0xA6;

/// `dspi_proto::generated::opcodes::REQ_SIGGEN_GET_STATUS`.
pub const REQ_SIGGEN_GET_STATUS: u8 = 0xA7;

/// `dspi_proto::generated::opcodes::REQ_SIGGEN_GET_CAPS`.
pub const REQ_SIGGEN_GET_CAPS: u8 = 0xA8;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_DEPTH`.
pub const REQ_SET_SUBHARM_DEPTH: u8 = 0xA9;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_DEPTH`.
pub const REQ_GET_SUBHARM_DEPTH: u8 = 0xAA;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_HOLD`.
pub const REQ_SET_SUBHARM_HOLD: u8 = 0xAB;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_HOLD`.
pub const REQ_GET_SUBHARM_HOLD: u8 = 0xAC;

/// `dspi_proto::generated::opcodes::REQ_SET_SUBHARM_CEILING`.
pub const REQ_SET_SUBHARM_CEILING: u8 = 0xAD;

/// `dspi_proto::generated::opcodes::REQ_GET_SUBHARM_CEILING`.
pub const REQ_GET_SUBHARM_CEILING: u8 = 0xAE;

/// `dspi_proto::generated::opcodes::REQ_GET_BUFFER_STATS`.
pub const REQ_GET_BUFFER_STATS: u8 = 0xB0;

/// `dspi_proto::generated::opcodes::REQ_RESET_BUFFER_STATS`.
pub const REQ_RESET_BUFFER_STATS: u8 = 0xB1;

/// `dspi_proto::generated::opcodes::REQ_GET_USB_ERROR_STATS`.
pub const REQ_GET_USB_ERROR_STATS: u8 = 0xB2;

/// `dspi_proto::generated::opcodes::REQ_RESET_USB_ERROR_STATS`.
pub const REQ_RESET_USB_ERROR_STATS: u8 = 0xB3;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_ENABLE`.
pub const REQ_SET_LEVELLER_ENABLE: u8 = 0xB4;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_ENABLE`.
pub const REQ_GET_LEVELLER_ENABLE: u8 = 0xB5;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_AMOUNT`.
pub const REQ_SET_LEVELLER_AMOUNT: u8 = 0xB6;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_AMOUNT`.
pub const REQ_GET_LEVELLER_AMOUNT: u8 = 0xB7;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_SPEED`.
pub const REQ_SET_LEVELLER_SPEED: u8 = 0xB8;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_SPEED`.
pub const REQ_GET_LEVELLER_SPEED: u8 = 0xB9;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_MAX_GAIN`.
pub const REQ_SET_LEVELLER_MAX_GAIN: u8 = 0xBA;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_MAX_GAIN`.
pub const REQ_GET_LEVELLER_MAX_GAIN: u8 = 0xBB;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_LOOKAHEAD`.
pub const REQ_SET_LEVELLER_LOOKAHEAD: u8 = 0xBC;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_LOOKAHEAD`.
pub const REQ_GET_LEVELLER_LOOKAHEAD: u8 = 0xBD;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_GATE`.
pub const REQ_SET_LEVELLER_GATE: u8 = 0xBE;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_GATE`.
pub const REQ_GET_LEVELLER_GATE: u8 = 0xBF;

/// `dspi_proto::generated::opcodes::REQ_SET_OUTPUT_TYPE`.
pub const REQ_SET_OUTPUT_TYPE: u8 = 0xC0;

/// `dspi_proto::generated::opcodes::REQ_GET_OUTPUT_TYPE`.
pub const REQ_GET_OUTPUT_TYPE: u8 = 0xC1;

/// `dspi_proto::generated::opcodes::REQ_SET_I2S_BCK_PIN`.
pub const REQ_SET_I2S_BCK_PIN: u8 = 0xC2;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_BCK_PIN`.
pub const REQ_GET_I2S_BCK_PIN: u8 = 0xC3;

/// `dspi_proto::generated::opcodes::REQ_SET_MCK_ENABLE`.
pub const REQ_SET_MCK_ENABLE: u8 = 0xC4;

/// `dspi_proto::generated::opcodes::REQ_GET_MCK_ENABLE`.
pub const REQ_GET_MCK_ENABLE: u8 = 0xC5;

/// `dspi_proto::generated::opcodes::REQ_SET_MCK_PIN`.
pub const REQ_SET_MCK_PIN: u8 = 0xC6;

/// `dspi_proto::generated::opcodes::REQ_GET_MCK_PIN`.
pub const REQ_GET_MCK_PIN: u8 = 0xC7;

/// `dspi_proto::generated::opcodes::REQ_SET_MCK_MULTIPLIER`.
pub const REQ_SET_MCK_MULTIPLIER: u8 = 0xC8;

/// `dspi_proto::generated::opcodes::REQ_GET_MCK_MULTIPLIER`.
pub const REQ_GET_MCK_MULTIPLIER: u8 = 0xC9;

/// `dspi_proto::generated::opcodes::REQ_SET_ADAT_ENABLE`.
pub const REQ_SET_ADAT_ENABLE: u8 = 0xCA;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_ENABLE`.
pub const REQ_GET_ADAT_ENABLE: u8 = 0xCB;

/// `dspi_proto::generated::opcodes::REQ_SET_ADAT_PIN`.
pub const REQ_SET_ADAT_PIN: u8 = 0xCC;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_PIN`.
pub const REQ_GET_ADAT_PIN: u8 = 0xCD;

/// `dspi_proto::generated::opcodes::REQ_GET_ADAT_STATUS`.
pub const REQ_GET_ADAT_STATUS: u8 = 0xCE;

/// `dspi_proto::generated::opcodes::REQ_SET_PREAMP_CH`.
pub const REQ_SET_PREAMP_CH: u8 = 0xD0;

/// `dspi_proto::generated::opcodes::REQ_GET_PREAMP_CH`.
pub const REQ_GET_PREAMP_CH: u8 = 0xD1;

/// `dspi_proto::generated::opcodes::REQ_SET_MASTER_VOLUME`.
pub const REQ_SET_MASTER_VOLUME: u8 = 0xD2;

/// `dspi_proto::generated::opcodes::REQ_GET_MASTER_VOLUME`.
pub const REQ_GET_MASTER_VOLUME: u8 = 0xD3;

/// `dspi_proto::generated::opcodes::REQ_SET_MASTER_VOLUME_MODE`.
pub const REQ_SET_MASTER_VOLUME_MODE: u8 = 0xD4;

/// `dspi_proto::generated::opcodes::REQ_GET_MASTER_VOLUME_MODE`.
pub const REQ_GET_MASTER_VOLUME_MODE: u8 = 0xD5;

/// `dspi_proto::generated::opcodes::REQ_SAVE_MASTER_VOLUME`.
pub const REQ_SAVE_MASTER_VOLUME: u8 = 0xD6;

/// `dspi_proto::generated::opcodes::REQ_GET_SAVED_MASTER_VOLUME`.
pub const REQ_GET_SAVED_MASTER_VOLUME: u8 = 0xD7;

/// `dspi_proto::generated::opcodes::REQ_SET_BAND_BYPASS`.
pub const REQ_SET_BAND_BYPASS: u8 = 0xD8;

/// `dspi_proto::generated::opcodes::REQ_GET_BAND_BYPASS`.
pub const REQ_GET_BAND_BYPASS: u8 = 0xD9;

/// `dspi_proto::generated::opcodes::REQ_SET_USER_VOLUME`.
pub const REQ_SET_USER_VOLUME: u8 = 0xDA;

/// `dspi_proto::generated::opcodes::REQ_GET_USER_VOLUME`.
pub const REQ_GET_USER_VOLUME: u8 = 0xDB;

/// `dspi_proto::generated::opcodes::REQ_SET_USER_MUTE`.
pub const REQ_SET_USER_MUTE: u8 = 0xDC;

/// `dspi_proto::generated::opcodes::REQ_GET_USER_MUTE`.
pub const REQ_GET_USER_MUTE: u8 = 0xDD;

/// `dspi_proto::generated::opcodes::REQ_SET_LEVELLER_MASKS`.
pub const REQ_SET_LEVELLER_MASKS: u8 = 0xDE;

/// `dspi_proto::generated::opcodes::REQ_GET_LEVELLER_MASKS`.
pub const REQ_GET_LEVELLER_MASKS: u8 = 0xDF;

/// `dspi_proto::generated::opcodes::REQ_SET_INPUT_SOURCE`.
pub const REQ_SET_INPUT_SOURCE: u8 = 0xE0;

/// `dspi_proto::generated::opcodes::REQ_GET_INPUT_SOURCE`.
pub const REQ_GET_INPUT_SOURCE: u8 = 0xE1;

/// `dspi_proto::generated::opcodes::REQ_GET_SPDIF_RX_STATUS`.
pub const REQ_GET_SPDIF_RX_STATUS: u8 = 0xE2;

/// `dspi_proto::generated::opcodes::REQ_GET_SPDIF_RX_CH_STATUS`.
pub const REQ_GET_SPDIF_RX_CH_STATUS: u8 = 0xE3;

/// `dspi_proto::generated::opcodes::REQ_SET_SPDIF_RX_PIN`.
pub const REQ_SET_SPDIF_RX_PIN: u8 = 0xE4;

/// `dspi_proto::generated::opcodes::REQ_GET_SPDIF_RX_PIN`.
pub const REQ_GET_SPDIF_RX_PIN: u8 = 0xE5;

/// `dspi_proto::generated::opcodes::REQ_SET_LG_SOUND_SYNC_ENABLE`.
pub const REQ_SET_LG_SOUND_SYNC_ENABLE: u8 = 0xE6;

/// `dspi_proto::generated::opcodes::REQ_GET_LG_SOUND_SYNC_ENABLE`.
pub const REQ_GET_LG_SOUND_SYNC_ENABLE: u8 = 0xE7;

/// `dspi_proto::generated::opcodes::REQ_GET_LG_SOUND_SYNC_STATUS`.
pub const REQ_GET_LG_SOUND_SYNC_STATUS: u8 = 0xE8;

/// `dspi_proto::generated::opcodes::REQ_SET_SPDIF_INPUT_ENABLE`.
pub const REQ_SET_SPDIF_INPUT_ENABLE: u8 = 0xE9;

/// `dspi_proto::generated::opcodes::REQ_SET_DAC_HW_MUTE_CONFIG`.
pub const REQ_SET_DAC_HW_MUTE_CONFIG: u8 = 0xEA;

/// `dspi_proto::generated::opcodes::REQ_GET_DAC_HW_MUTE_CONFIG`.
pub const REQ_GET_DAC_HW_MUTE_CONFIG: u8 = 0xEB;

/// `dspi_proto::generated::opcodes::REQ_TEST_DAC_HW_MUTE`.
pub const REQ_TEST_DAC_HW_MUTE: u8 = 0xEC;

/// `dspi_proto::generated::opcodes::REQ_SET_INPUT_RATE`.
pub const REQ_SET_INPUT_RATE: u8 = 0xED;

/// `dspi_proto::generated::opcodes::REQ_GET_INPUT_RATE`.
pub const REQ_GET_INPUT_RATE: u8 = 0xEE;

/// `dspi_proto::generated::opcodes::REQ_GET_SPDIF_INPUT_CONFIG`.
pub const REQ_GET_SPDIF_INPUT_CONFIG: u8 = 0xEF;

/// `dspi_proto::generated::opcodes::REQ_ENTER_BOOTLOADER`.
pub const REQ_ENTER_BOOTLOADER: u8 = 0xF0;

/// `dspi_proto::generated::opcodes::REQ_SET_I2S_RX_PIN`.
pub const REQ_SET_I2S_RX_PIN: u8 = 0xF1;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_RX_PIN`.
pub const REQ_GET_I2S_RX_PIN: u8 = 0xF2;

/// `dspi_proto::generated::opcodes::REQ_SET_I2S_INPUT_CHANNELS`.
pub const REQ_SET_I2S_INPUT_CHANNELS: u8 = 0xF3;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_INPUT_CHANNELS`.
pub const REQ_GET_I2S_INPUT_CHANNELS: u8 = 0xF4;

/// `dspi_proto::generated::opcodes::REQ_SET_UART_CONFIG`.
pub const REQ_SET_UART_CONFIG: u8 = 0xF5;

/// `dspi_proto::generated::opcodes::REQ_GET_UART_CONFIG`.
pub const REQ_GET_UART_CONFIG: u8 = 0xF6;

/// `dspi_proto::generated::opcodes::REQ_SET_I2C_CONFIG`.
pub const REQ_SET_I2C_CONFIG: u8 = 0xF7;

/// `dspi_proto::generated::opcodes::REQ_GET_I2C_CONFIG`.
pub const REQ_GET_I2C_CONFIG: u8 = 0xF8;

/// `dspi_proto::generated::opcodes::REQ_GET_CTRL_IFACE_STATUS`.
pub const REQ_GET_CTRL_IFACE_STATUS: u8 = 0xF9;

/// `dspi_proto::generated::opcodes::REQ_SET_LOUDNESS_MASK`.
pub const REQ_SET_LOUDNESS_MASK: u8 = 0xFA;

/// `dspi_proto::generated::opcodes::REQ_GET_LOUDNESS_MASK`.
pub const REQ_GET_LOUDNESS_MASK: u8 = 0xFB;

/// `dspi_proto::generated::opcodes::REQ_SET_CROSSFEED_OUTPUTS`.
pub const REQ_SET_CROSSFEED_OUTPUTS: u8 = 0xFC;

/// `dspi_proto::generated::opcodes::REQ_GET_CROSSFEED_OUTPUTS`.
pub const REQ_GET_CROSSFEED_OUTPUTS: u8 = 0xFD;

/// `dspi_proto::generated::opcodes::REQ_SET_I2S_CLOCK_PIN_MODE`.
pub const REQ_SET_I2S_CLOCK_PIN_MODE: u8 = 0xFE;

/// `dspi_proto::generated::opcodes::REQ_GET_I2S_CLOCK_PIN_MODE`.
pub const REQ_GET_I2S_CLOCK_PIN_MODE: u8 = 0xFF;

// ── status ── Status bytes and codes the calls return

/// `dspi_proto::generated::status::PIN_CONFIG_SUCCESS`.
pub const PIN_CONFIG_SUCCESS: u8 = 0;

/// `dspi_proto::generated::status::PIN_CONFIG_INVALID_PIN`.
pub const PIN_CONFIG_INVALID_PIN: u8 = 1;

/// `dspi_proto::generated::status::PIN_CONFIG_PIN_IN_USE`.
pub const PIN_CONFIG_PIN_IN_USE: u8 = 2;

/// `dspi_proto::generated::status::PIN_CONFIG_INVALID_OUTPUT`.
pub const PIN_CONFIG_INVALID_OUTPUT: u8 = 3;

/// `dspi_proto::generated::status::PIN_CONFIG_OUTPUT_ACTIVE`.
pub const PIN_CONFIG_OUTPUT_ACTIVE: u8 = 4;

/// `dspi_proto::generated::status::PIN_CONFIG_INVALID_PARAM`.
pub const PIN_CONFIG_INVALID_PARAM: u8 = 5;

/// `PIN_RESET_TO_DEFAULT` in the firmware's config.h.
pub const PIN_RESET_TO_DEFAULT: u8 = 0xFF;

/// `dspi_proto::generated::status::PRESET_OK`.
pub const PRESET_OK: u8 = 0;

/// `dspi_proto::generated::status::PRESET_ERR_INVALID_SLOT`.
pub const PRESET_ERR_INVALID_SLOT: u8 = 1;

/// `dspi_proto::generated::status::PRESET_ERR_SLOT_EMPTY`.
pub const PRESET_ERR_SLOT_EMPTY: u8 = 2;

/// `dspi_proto::generated::status::PRESET_ERR_CRC`.
pub const PRESET_ERR_CRC: u8 = 3;

/// `dspi_proto::generated::status::PRESET_ERR_FLASH_WRITE`.
pub const PRESET_ERR_FLASH_WRITE: u8 = 4;

/// `dspi_core::consts::FLASH_OK`.
pub const FLASH_OK: u8 = 0;

/// `dspi_core::consts::FLASH_ERR_WRITE`.
pub const FLASH_ERR_WRITE: u8 = 1;

/// `dspi_core::consts::FLASH_ERR_NO_DATA`.
pub const FLASH_ERR_NO_DATA: u8 = 2;

/// `dspi_core::consts::FLASH_ERR_CRC`.
pub const FLASH_ERR_CRC: u8 = 3;

/// `dspi_core::consts::COMPAT_UNKNOWN`.
pub const COMPAT_UNKNOWN: u8 = 0;

/// `dspi_core::consts::COMPAT_OK`.
pub const COMPAT_OK: u8 = 1;

/// `dspi_core::consts::COMPAT_FIRMWARE_TOO_OLD`.
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;

/// `dspi_core::consts::COMPAT_FIRMWARE_TOO_NEW`.
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

/// `dspi_core::consts::FW_BETA_EARLY`.
pub const FW_BETA_EARLY: u8 = 0xFF;

// ── wire ── The bulk parameter image (WireBulkParams)

/// `dspi_proto::generated::wire::WIRE_FORMAT_VERSION`.
pub const WIRE_FORMAT_VERSION: u8 = 32;

/// `dspi_proto::wire_compat::MIN_WIRE_FORMAT_VERSION`.
pub const MIN_WIRE_FORMAT_VERSION: u8 = 16;

/// `dspi_proto::wire_compat::feat::LEVELLER_MASKS`.
pub const WIRE_FEAT_LEVELLER_MASKS: u32 = 1 << 1;

/// `dspi_proto::wire_compat::feat::LOUDNESS_MASK`.
pub const WIRE_FEAT_LOUDNESS_MASK: u32 = 1 << 2;

/// `dspi_proto::wire_compat::feat::CROSSFEED_MASK`.
pub const WIRE_FEAT_CROSSFEED_MASK: u32 = 1 << 3;

/// `dspi_proto::wire_compat::feat::UPMIX_PRESENCE`.
pub const WIRE_FEAT_UPMIX_PRESENCE: u32 = 1 << 9;

/// `dspi_proto::wire_compat::feat::SUBHARM_EXT`.
pub const WIRE_FEAT_SUBHARM_EXT: u32 = 1 << 12;

/// `dspi_proto::wire_compat::feat::PSYBASS`.
pub const WIRE_FEAT_PSYBASS: u32 = 1 << 6;

/// `dspi_proto::wire_compat::feat::UPMIX`.
pub const WIRE_FEAT_UPMIX: u32 = 1 << 8;

/// `dspi_proto::wire_compat::feat::SUBHARM`.
pub const WIRE_FEAT_SUBHARM: u32 = 1 << 11;

/// `dspi_proto::wire_compat::feat::TUBE`.
pub const WIRE_FEAT_TUBE: u32 = 1 << 13;

/// `dspi_proto::wire_compat::feat::LIMITER`.
pub const WIRE_FEAT_LIMITER: u32 = 1 << 14;

/// `dspi_proto::generated::BULK_SIZE`.
pub const BULK_PARAMS_SIZE: usize = 6136;

/// Bytes per chunked bulk-image transfer (`REQ_*_ALL_PARAMS_CHUNK`).
pub const BULK_CHUNK_SIZE: usize = 2048;

/// Offset of the `header` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_HEADER: usize = 0;

/// Offset of the `global` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_GLOBAL: usize = 16;

/// Offset of the `crossfeed` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_CROSSFEED: usize = 32;

/// Offset of the `legacy` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_LEGACY: usize = 48;

/// Offset of the `delays` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_DELAYS: usize = 64;

/// Offset of the `crosspoints` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_CROSSPOINTS: usize = 132;

/// Offset of the `outputs` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_OUTPUTS: usize = 708;

/// Offset of the `pins` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_PINS: usize = 816;

/// Offset of the `eq` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_EQ: usize = 824;

/// Offset of the `channel_names` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_CHANNEL_NAMES: usize = 4088;

/// Offset of the `i2s_config` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_I2S: usize = 4632;

/// Offset of the `leveller` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_LEVELLER: usize = 4648;

/// Offset of the `preamp` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_PREAMP: usize = 4668;

/// Offset of the `master_volume` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_MASTER_VOLUME: usize = 4700;

/// Offset of the `input_config` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_INPUT_CONFIG: usize = 4716;

/// Offset of the `lg_sound_sync` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_LG_SOUND_SYNC: usize = 4732;

/// Offset of the `user_volume` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_USER_VOLUME: usize = 4748;

/// Offset of the `dac_hw_mute` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_DAC_HW_MUTE: usize = 4764;

/// Offset of the `crossovers` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_CROSSOVER: usize = 4780;

/// Offset of the `adat_config` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_ADAT: usize = 5868;

/// Offset of the `psybass` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_PSYBASS: usize = 5876;

/// Offset of the `upmix` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_UPMIX: usize = 5900;

/// Offset of the `subharm` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_SUBHARM: usize = 5944;

/// Offset of the `tube` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_TUBE: usize = 5980;

/// Offset of the `limiter` section (`dspi_proto::generated::SECTIONS`).
pub const OFF_LIMITER: usize = 6028;

// ── dims ── Array sizes of the records

/// `dspi_core::consts::MAX_CHANNELS`.
pub const MAX_CHANNELS: usize = 17;

/// `dspi_core::consts::MAX_INPUTS`.
pub const MAX_INPUTS: usize = 8;

/// `dspi_core::consts::MAX_OUTPUTS`.
pub const MAX_OUTPUTS: usize = 9;

/// `dspi_core::consts::BANDS_PER_CHANNEL`.
pub const BANDS_PER_CHANNEL: usize = 10;

/// `dspi_core::consts::FIRMWARE_BANDS_PER_CHANNEL`.
pub const FIRMWARE_BANDS_PER_CHANNEL: usize = 12;

/// `dspi_core::consts::MAX_XOVER_BANDS`.
pub const MAX_XOVER_BANDS: usize = 4;

/// `dspi_proto::generated::filter::XOVER_BAND_BASE`.
pub const XOVER_BAND_BASE: u8 = 20;

/// `dspi_core::consts::MAX_PRESETS`.
pub const MAX_PRESETS: usize = 10;

/// `dspi_core::consts::CHANNEL_NAME_LEN`.
pub const CHANNEL_NAME_LEN: usize = 32;

/// `dspi_core::consts::MAX_PHYSICAL_OUTPUTS`.
pub const MAX_PHYSICAL_OUTPUTS: usize = 5;

// ── eq ── Filter types and response curves

/// `dspi_proto::FilterType::Flat`.
pub const FILTER_FLAT: u8 = 0;

/// `dspi_proto::FilterType::Peaking`.
pub const FILTER_PEAKING: u8 = 1;

/// `dspi_proto::FilterType::LowShelf`.
pub const FILTER_LOWSHELF: u8 = 2;

/// `dspi_proto::FilterType::HighShelf`.
pub const FILTER_HIGHSHELF: u8 = 3;

/// `dspi_proto::FilterType::LowPass`.
pub const FILTER_LOWPASS: u8 = 4;

/// `dspi_proto::FilterType::HighPass`.
pub const FILTER_HIGHPASS: u8 = 5;

/// `dspi_proto::FilterType::Notch`.
pub const FILTER_NOTCH: u8 = 6;

/// `dspi_proto::FilterType::AllPass`.
pub const FILTER_ALLPASS: u8 = 7;

/// `dspi_proto::FilterType::AllPass1`.
pub const FILTER_ALLPASS1: u8 = 8;

/// `dspi_proto::FilterType::LowShelf1`.
pub const FILTER_LOWSHELF1: u8 = 9;

/// `dspi_proto::FilterType::HighShelf1`.
pub const FILTER_HIGHSHELF1: u8 = 10;

/// `dspi_proto::FilterType::LinkwitzTransform`.
pub const FILTER_LINKWITZ_TRANSFORM: u8 = 11;

/// `dspi_proto::FilterType::LowPass1`.
pub const FILTER_LOWPASS1: u8 = 12;

/// `dspi_proto::FilterType::HighPass1`.
pub const FILTER_HIGHPASS1: u8 = 13;

/// `FilterType::XOVER_FIRST`.
pub const FILTER_XOVER_FIRST: u8 = 32;

/// `FilterType::XOVER_LAST`.
pub const FILTER_XOVER_LAST: u8 = 63;

/// `dspi_core::consts::DEFAULT_Q`.
pub const DEFAULT_Q: f32 = 0.707;

/// `dspi_proto::dsp::SAMPLE_RATE`.
pub const SAMPLE_RATE: f64 = 48000.0;

/// `dspi_core::CURVE_POINTS`.
pub const MAGNITUDE_POINTS: usize = 201;

// ── volume ── Master volume

/// `MASTER_VOL_MUTE_DB` in the firmware's config.h.
pub const MASTER_VOL_MUTE_DB: f32 = -128.0;

// ── effects ── Parameter ids of the effect blocks

/// `dspi_core::consts::PSYBASS_PARAM_CUTOFF`.
pub const PSYBASS_PARAM_CUTOFF: u8 = 0;

/// `dspi_core::consts::PSYBASS_PARAM_HARMONICS`.
pub const PSYBASS_PARAM_HARMONICS: u8 = 1;

/// `dspi_core::consts::PSYBASS_PARAM_DRIVE`.
pub const PSYBASS_PARAM_DRIVE: u8 = 2;

/// `dspi_core::consts::PSYBASS_PARAM_CHARACTER`.
pub const PSYBASS_PARAM_CHARACTER: u8 = 3;

/// `dspi_core::consts::PSYBASS_PARAM_ORIGINAL`.
pub const PSYBASS_PARAM_ORIGINAL: u8 = 4;

/// `dspi_core::consts::UPMIX_PARAM_ENABLED`.
pub const UPMIX_PARAM_ENABLED: u8 = 0;

/// `dspi_core::consts::UPMIX_PARAM_CENTER_MODE`.
pub const UPMIX_PARAM_CENTER_MODE: u8 = 1;

/// `dspi_core::consts::UPMIX_PARAM_SURROUND_MODE`.
pub const UPMIX_PARAM_SURROUND_MODE: u8 = 2;

/// `dspi_core::consts::UPMIX_PARAM_STRENGTH`.
pub const UPMIX_PARAM_STRENGTH: u8 = 3;

/// `dspi_core::consts::UPMIX_PARAM_WIDTH`.
pub const UPMIX_PARAM_WIDTH: u8 = 4;

/// `dspi_core::consts::UPMIX_PARAM_THRESHOLD`.
pub const UPMIX_PARAM_THRESHOLD: u8 = 5;

/// `dspi_core::consts::UPMIX_PARAM_ATTACK`.
pub const UPMIX_PARAM_ATTACK: u8 = 6;

/// `dspi_core::consts::UPMIX_PARAM_RELEASE`.
pub const UPMIX_PARAM_RELEASE: u8 = 7;

/// `dspi_core::consts::UPMIX_PARAM_DETECTOR_HPF`.
pub const UPMIX_PARAM_DETECTOR_HPF: u8 = 8;

/// `dspi_core::consts::UPMIX_PARAM_SURROUND_DELAY`.
pub const UPMIX_PARAM_SURROUND_DELAY: u8 = 9;

/// `dspi_core::consts::UPMIX_PARAM_SURROUND_HPF`.
pub const UPMIX_PARAM_SURROUND_HPF: u8 = 10;

/// `dspi_core::consts::UPMIX_PARAM_SURROUND_LPF`.
pub const UPMIX_PARAM_SURROUND_LPF: u8 = 11;

/// `dspi_core::consts::UPMIX_PARAM_DECORRELATION`.
pub const UPMIX_PARAM_DECORRELATION: u8 = 12;

/// `dspi_core::consts::UPMIX_PARAM_PRESENCE`.
pub const UPMIX_PARAM_PRESENCE: u8 = 13;

/// `dspi_core::consts::UPMIX_PARAM_COUNT`.
pub const UPMIX_PARAM_COUNT: usize = 14;

/// `dspi_core::consts::SUBHARM_PARAM_ENABLED`.
pub const SUBHARM_PARAM_ENABLED: u8 = 0;

/// `dspi_core::consts::SUBHARM_PARAM_MASK`.
pub const SUBHARM_PARAM_MASK: u8 = 1;

/// `dspi_core::consts::SUBHARM_PARAM_LOW`.
pub const SUBHARM_PARAM_LOW: u8 = 2;

/// `dspi_core::consts::SUBHARM_PARAM_HIGH`.
pub const SUBHARM_PARAM_HIGH: u8 = 3;

/// `dspi_core::consts::SUBHARM_PARAM_TOP`.
pub const SUBHARM_PARAM_TOP: u8 = 4;

/// `dspi_core::consts::SUBHARM_PARAM_BOOST`.
pub const SUBHARM_PARAM_BOOST: u8 = 5;

/// `dspi_core::consts::SUBHARM_PARAM_SELECT`.
pub const SUBHARM_PARAM_SELECT: u8 = 6;

/// `dspi_core::consts::SUBHARM_PARAM_DEPTH`.
pub const SUBHARM_PARAM_DEPTH: u8 = 7;

/// `dspi_core::consts::SUBHARM_PARAM_HOLD`.
pub const SUBHARM_PARAM_HOLD: u8 = 8;

/// `dspi_core::consts::SUBHARM_PARAM_CEILING`.
pub const SUBHARM_PARAM_CEILING: u8 = 9;

/// `dspi_core::consts::SUBHARM_PARAM_LINK`.
pub const SUBHARM_PARAM_LINK: u8 = 10;

/// `dspi_core::consts::SUBHARM_PARAM_COUNT`.
pub const SUBHARM_PARAM_COUNT: usize = 11;

/// `dspi_proto::generated::tube::TUBE_PARAM_ENABLED`.
pub const TUBE_PARAM_ENABLED: u8 = 0;

/// `dspi_proto::generated::tube::TUBE_PARAM_OUTPUT_MASK`.
pub const TUBE_PARAM_MASK: u8 = 1;

/// `dspi_proto::generated::tube::TUBE_PARAM_TUBE_TYPE`.
pub const TUBE_PARAM_TYPE: u8 = 2;

/// `dspi_proto::generated::tube::TUBE_PARAM_DRIVE_DB`.
pub const TUBE_PARAM_DRIVE: u8 = 3;

/// `dspi_proto::generated::tube::TUBE_PARAM_BIAS_PCT`.
pub const TUBE_PARAM_BIAS: u8 = 4;

/// `dspi_proto::generated::tube::TUBE_PARAM_ASYM_DB`.
pub const TUBE_PARAM_ASYM: u8 = 5;

/// `dspi_proto::generated::tube::TUBE_PARAM_HARDNESS_PCT`.
pub const TUBE_PARAM_HARDNESS: u8 = 6;

/// `dspi_proto::generated::tube::TUBE_PARAM_SAG_PCT`.
pub const TUBE_PARAM_SAG: u8 = 7;

/// `dspi_proto::generated::tube::TUBE_PARAM_RECTIFIER`.
pub const TUBE_PARAM_RECTIFIER: u8 = 8;

/// `dspi_proto::generated::tube::TUBE_PARAM_XFMR_ENABLED`.
pub const TUBE_PARAM_XFMR: u8 = 9;

/// `dspi_proto::generated::tube::TUBE_PARAM_XFMR_DAMPING`.
pub const TUBE_PARAM_DAMPING: u8 = 10;

/// `dspi_proto::generated::tube::TUBE_PARAM_XFMR_RES_HZ`.
pub const TUBE_PARAM_RESONANCE: u8 = 11;

/// `dspi_proto::generated::tube::TUBE_PARAM_MIX_PCT`.
pub const TUBE_PARAM_MIX: u8 = 12;

/// `dspi_proto::generated::tube::TUBE_PARAM_TRIM_DB`.
pub const TUBE_PARAM_TRIM: u8 = 13;

/// `dspi_core::consts::TUBE_PARAM_COUNT`.
pub const TUBE_PARAM_COUNT: usize = 14;

/// `dspi_proto::generated::limiter::LIMITER_PARAM_ENABLED`.
pub const LIMITER_PARAM_ENABLED: u8 = 0;

/// `dspi_proto::generated::limiter::LIMITER_PARAM_THRESHOLD_DB`.
pub const LIMITER_PARAM_THRESHOLD_DB: u8 = 1;

/// `dspi_proto::generated::limiter::LIMITER_PARAM_RELEASE_MS`.
pub const LIMITER_PARAM_RELEASE_MS: u8 = 2;

/// `dspi_proto::generated::limiter::LIMITER_PARAM_LINK_GROUP`.
pub const LIMITER_PARAM_LINK_GROUP: u8 = 3;

/// `dspi_proto::generated::limiter::LIMITER_GET_METER`.
pub const LIMITER_GET_METER: u8 = 0x80;

/// `dspi_proto::generated::limiter::LIMITER_GET_STATUS`.
pub const LIMITER_GET_STATUS: u8 = 0x81;

/// `dspi_proto::generated::limiter::LIMITER_ALL_OUTPUTS`.
pub const LIMITER_ALL_OUTPUTS: u8 = 0xFF;

// ── rta ── The analyser

/// `dspi_proto::packets::RTA_VERSION`.
pub const RTA_VERSION: u8 = 3;

/// `dspi_core::consts::RTA_MAX_BANDS`.
pub const RTA_MAX_BANDS: usize = 37;

/// `dspi_core::consts::RTA_MAX_CHANNELS`.
pub const RTA_MAX_CHANNELS: usize = 16;

/// `dspi_core::consts::RTA_MAX_BINS`.
pub const RTA_MAX_BINS: usize = 512;

/// `dspi_core::consts::RTA_TAP_INPUT`.
pub const RTA_TAP_INPUT: u8 = 0;

/// `dspi_core::consts::RTA_TAP_OUTPUT`.
pub const RTA_TAP_OUTPUT: u8 = 1;

/// `dspi_core::consts::RTA_FLOOR_DB`.
pub const RTA_FLOOR_DB: f32 = -121.5;

// ── siggen ── The signal generator

/// `dspi_core::consts::SIGGEN_VERSION`.
pub const SIGGEN_VERSION: u8 = 1;

/// `dspi_core::consts::SIGGEN_TYPE_COUNT`.
pub const SIGGEN_TYPE_COUNT: usize = 15;

/// `dspi_core::consts::SIGGEN_FLAG_RAW`.
pub const SIGGEN_FLAG_RAW: u8 = 0x01;

/// `dspi_core::consts::SIGGEN_FLAG_DECORR`.
pub const SIGGEN_FLAG_DECORR: u8 = 0x02;

/// `dspi_core::consts::SIGGEN_FLAG_WALK`.
pub const SIGGEN_FLAG_WALK: u8 = 0x04;

/// `dspi_core::consts::SIGGEN_CTL_STOP`.
pub const SIGGEN_CTL_STOP: u16 = 0;

/// `dspi_core::consts::SIGGEN_CTL_START`.
pub const SIGGEN_CTL_START: u16 = 1;

/// `dspi_core::consts::SIGGEN_CTL_STOP_NOW`.
pub const SIGGEN_CTL_STOP_NOW: u16 = 2;

// ── stats ── Statistics ids (`dspi_fetch_stat`)

/// `dspi_core::consts::STAT_PDM_RING_OVERRUNS`.
pub const STAT_PDM_RING_OVERRUNS: u16 = 3;

/// `dspi_core::consts::STAT_PDM_RING_UNDERRUNS`.
pub const STAT_PDM_RING_UNDERRUNS: u16 = 4;

/// `dspi_core::consts::STAT_PDM_DMA_OVERRUNS`.
pub const STAT_PDM_DMA_OVERRUNS: u16 = 5;

/// `dspi_core::consts::STAT_PDM_DMA_UNDERRUNS`.
pub const STAT_PDM_DMA_UNDERRUNS: u16 = 6;

/// `dspi_core::consts::STAT_SPDIF_OVERRUNS`.
pub const STAT_SPDIF_OVERRUNS: u16 = 7;

/// `dspi_core::consts::STAT_SPDIF_UNDERRUNS`.
pub const STAT_SPDIF_UNDERRUNS: u16 = 8;

/// `dspi_core::consts::STAT_CLOCK_HZ`.
pub const STAT_CLOCK_HZ: u16 = 13;

/// `dspi_core::consts::STAT_CORE_MV`.
pub const STAT_CORE_MV: u16 = 14;

/// `dspi_core::consts::STAT_SAMPLE_RATE`.
pub const STAT_SAMPLE_RATE: u16 = 15;

/// `dspi_core::consts::STAT_TEMPERATURE`.
pub const STAT_TEMPERATURE: u16 = 16;

/// `dspi_core::consts::STAT_STARVATION_TOTAL`.
pub const STAT_STARVATION_TOTAL: u16 = 17;

/// `dspi_core::consts::STAT_STARVATION_FIRST`.
pub const STAT_STARVATION_FIRST: u16 = 18;

/// `dspi_core::consts::STAT_USB_RING_OVERRUNS`.
pub const STAT_USB_RING_OVERRUNS: u16 = 22;

// ── notify ── Bits of `NotifyResult::flags`

/// `dspi_core::consts::NOTIFY_STATE`.
pub const NOTIFY_STATE: u32 = 1 << 0;

/// `dspi_core::consts::NOTIFY_CURVES`.
pub const NOTIFY_CURVES: u32 = 1 << 1;

/// `dspi_core::consts::NOTIFY_REFRESHED`.
pub const NOTIFY_REFRESHED: u32 = 1 << 2;

/// `dspi_core::consts::NOTIFY_INPUT_FORMAT`.
pub const NOTIFY_INPUT_FORMAT: u32 = 1 << 3;

/// `dspi_core::consts::NOTIFY_PRESET`.
pub const NOTIFY_PRESET: u32 = 1 << 4;

/// `dspi_core::consts::NOTIFY_SIGGEN`.
pub const NOTIFY_SIGGEN: u32 = 1 << 5;

/// `dspi_core::consts::NOTIFY_CS_AUX`.
pub const NOTIFY_CS_AUX: u32 = 1 << 6;

/// `dspi_core::consts::NOTIFY_IR_LEARN`.
pub const NOTIFY_IR_LEARN: u32 = 1 << 7;

/// `dspi_core::consts::NOTIFY_HISTORY`.
pub const NOTIFY_HISTORY: u32 = 1 << 12;

// ── files ── Configuration and filter files

/// `schemaVersion` of the configuration documents dspi-core writes.
pub const SCHEMA_VERSION: i64 = 1;

/// `dspi_session::filterfile::FORMAT_VERSION`.
pub const FORMAT_VERSION: u32 = 2;

/// `dspi_core::consts::IMPORT_VOLUMES`.
pub const IMPORT_VOLUMES: u32 = 1;

/// `dspi_core::consts::IMPORT_HARDWARE`.
pub const IMPORT_HARDWARE: u32 = 2;

/// `dspi_core::consts::IMPORT_EQ`.
pub const IMPORT_EQ: u32 = 4;

/// `dspi_core::consts::IMPORT_CROSSOVER`.
pub const IMPORT_CROSSOVER: u32 = 8;

/// `dspi_core::consts::IMPORT_ROUTING`.
pub const IMPORT_ROUTING: u32 = 16;

/// `dspi_core::consts::IMPORT_OUTPUTS`.
pub const IMPORT_OUTPUTS: u32 = 32;

/// `dspi_core::consts::IMPORT_LIMITERS`.
pub const IMPORT_LIMITERS: u32 = 64;

/// `dspi_core::consts::IMPORT_EFFECTS`.
pub const IMPORT_EFFECTS: u32 = 128;

/// `dspi_core::consts::IMPORT_NAMES`.
pub const IMPORT_NAMES: u32 = 256;

/// `dspi_core::consts::IMPORT_ALL_BLOCKS`.
pub const IMPORT_ALL_BLOCKS: u32 = 511;

/// `dspi_core::consts::IMPORT_BY_BLOCK`.
pub const IMPORT_BY_BLOCK: u32 = 65536;

// ── cs ── Control surface table sizes

/// `dspi_proto::generated::cs::CS_MAX_BINDINGS`.
pub const CS_MAX_BINDINGS: usize = 16;

/// `dspi_proto::generated::cs::CS_MAX_IR_COMMANDS`.
pub const CS_MAX_IR: usize = 16;

/// `dspi_proto::generated::cs::CS_MAX_GROUPS`.
pub const CS_MAX_GROUPS: usize = 8;

/// `dspi_proto::generated::cs::CS_MAX_MACROS`.
pub const CS_MAX_MACROS: usize = 8;

/// `dspi_proto::generated::cs::CS_MAX_MACRO_STEPS`.
pub const CS_MAX_STEPS: usize = 8;

/// `dspi_proto::generated::cs::CS_MAX_DISPLAY_PAGES`.
pub const CS_MAX_PAGES: usize = 16;
