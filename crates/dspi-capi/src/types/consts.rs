//! The named constants of the C header, copied value for value from the old
//! desktop core (`tests/data/dspi_core_old.h`, every `pub const` cbindgen put
//! into its `dspi_core.h`), so the Qt app compiles unchanged (WP Q1).
//!
//! cbindgen only sees literals, so these stay literals. The tests in
//! `types/tests_wp_q1.rs` check them against the firmware-generated constants
//! (`dspi_proto::generated`) and `dspi_core::consts` wherever those name the
//! same thing, and the header test checks every `#define` against the old
//! header. Grouped by the old core's module; each group's first constant
//! carries the group's comment.

// ── dsp_math.rs ────────────────────────────────────────────────────

/// Filter-response math: the sample rate and the curve size of the response
/// functions.
pub const SAMPLE_RATE: f64 = 48000.0;
/// Number of magnitude curve points (log-spaced from 10 Hz to 20 kHz).
pub const MAGNITUDE_POINTS: usize = 201;

// ── notify.rs ──────────────────────────────────────────────────────

/// Bulk IN notification endpoint on the vendor interface.
pub const NOTIFY_ENDPOINT: u8 = 0x83;

/// `NotifyResult.flags` bits: what a batch of notifications changed.
///
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

/// USB Device IDs.
///
/// DSPi vendor ID. Firmware moved from 0x2E8A (Raspberry Pi's, still used by
/// the RP2040/RP2350 ROM bootloader) to 0x2E8B in May 2026.
pub const VENDOR_ID: u16 = 0x2e8b;
/// Vendor ID used by firmware before the change; still recognised so an old
/// device shows up and can be told it needs a firmware update.
pub const LEGACY_VENDOR_ID: u16 = 0x2e8a;
/// USB product id of a DSPi.
pub const PRODUCT_ID: u16 = 0xfeaa;
/// Vendor interface number (WinUSB / WCID interface).
pub const VENDOR_INTERFACE: u8 = 2;

/// bmRequestType.
///
/// Host→Device | Vendor | Interface
pub const REQ_TYPE_OUT: u8 = 0x41;
/// Device→Host | Vendor | Interface
pub const REQ_TYPE_IN: u8 = 0xC1;

/// Control Surfaces aux outputs.
pub const REQ_SET_CS_AUX_STATE: u8 = 0x04;
/// wValue = slot: 1 byte (STALL if not an aux);
/// wValue = 0xFFFF: 48 bytes {state[16],
/// level_q8[16] LE}, zeros on non-aux slots
pub const REQ_GET_CS_AUX_STATE: u8 = 0x05;
/// wValue = slot, 2 bytes 8.8 percent LE, clamped
/// to 100 %; STALL unless a CS_TYPE_AUX_PWM slot
pub const REQ_SET_CS_AUX_LEVEL: u8 = 0x06;
/// wValue = slot; 2 bytes 8.8 percent LE (0 on
/// AUX_OUT; STALL if not an aux)
pub const REQ_GET_CS_AUX_LEVEL: u8 = 0x07;

/// Spectrum analyser (RTA).
pub const REQ_RTA_SET_CONFIG: u8 = 0x08;
/// returns RtaConfig (12 B)
pub const REQ_RTA_GET_CONFIG: u8 = 0x09;
/// wValue 0 = RtaCaps (16 B); 1.. = band centre chunks
pub const REQ_RTA_GET_CAPS: u8 = 0x0A;
/// wValue = channel; returns RtaBandFrame (82 B)
pub const REQ_RTA_GET_BANDS: u8 = 0x0B;
/// wValue = byte offset into the bin frame
pub const REQ_RTA_GET_BINS: u8 = 0x0C;
/// returns RtaStatus (24 B)
pub const REQ_RTA_GET_STATUS: u8 = 0x0D;
/// wValue = RTA_CTL_*; returns 1 byte
pub const REQ_RTA_CONTROL: u8 = 0x0E;
/// USB only: every live channel's RtaBandFrame
pub const REQ_RTA_GET_BANDS_ALL: u8 = 0x0F;

/// Subharmonic synthesizer.
pub const REQ_SET_SUBHARM: u8 = 0x10;
/// Reads what `REQ_SET_SUBHARM` writes: 1 byte 0/1
pub const REQ_GET_SUBHARM: u8 = 0x11;
/// float dB, 24-36 Hz band level
pub const REQ_SET_SUBHARM_LOW: u8 = 0x12;
/// Reads what `REQ_SET_SUBHARM_LOW` writes: float dB, 24-36 Hz band level
pub const REQ_GET_SUBHARM_LOW: u8 = 0x13;
/// float dB, 36-56 Hz band level
pub const REQ_SET_SUBHARM_HIGH: u8 = 0x14;
/// Reads what `REQ_SET_SUBHARM_HIGH` writes: float dB, 36-56 Hz band level
pub const REQ_GET_SUBHARM_HIGH: u8 = 0x15;
/// float dB, LF boost bell
pub const REQ_SET_SUBHARM_BOOST: u8 = 0x16;
/// Reads what `REQ_SET_SUBHARM_BOOST` writes: float dB, LF boost bell
pub const REQ_GET_SUBHARM_BOOST: u8 = 0x17;
/// uint16 LE output mask
pub const REQ_SET_SUBHARM_MASK: u8 = 0x18;
/// Reads what `REQ_SET_SUBHARM_MASK` writes: uint16 LE output mask
pub const REQ_GET_SUBHARM_MASK: u8 = 0x19;
/// float dB: preamp headroom to free
pub const REQ_GET_SUBHARM_HEADROOM: u8 = 0x1A;
/// float dB, 56-80 Hz band level
pub const REQ_SET_SUBHARM_TOP: u8 = 0x1B;
/// Reads what `REQ_SET_SUBHARM_TOP` writes: float dB, 56-80 Hz band level
pub const REQ_GET_SUBHARM_TOP: u8 = 0x1C;
/// 1 byte SUBHARM_SELECT_* mode
pub const REQ_SET_SUBHARM_SELECT: u8 = 0x1D;
/// Reads what `REQ_SET_SUBHARM_SELECT` writes: 1 byte SUBHARM_SELECT_* mode
pub const REQ_GET_SUBHARM_SELECT: u8 = 0x1E;
/// NUM_OUTPUT_CHANNELS x uint16 LE sub peaks
pub const REQ_GET_SUBHARM_METER: u8 = 0x1F;
/// 1 byte 0/1; runtime only, never persisted
pub const REQ_SET_SUBHARM_SOLO: u8 = 0x2C;
/// Reads what `REQ_SET_SUBHARM_SOLO` writes: 1 byte 0/1; runtime only, never persisted
pub const REQ_GET_SUBHARM_SOLO: u8 = 0x2D;
/// 1 byte 0/1, pair-linked synthesis
pub const REQ_SET_SUBHARM_LINK: u8 = 0x2E;
/// Reads what `REQ_SET_SUBHARM_LINK` writes: 1 byte 0/1, pair-linked synthesis
pub const REQ_GET_SUBHARM_LINK: u8 = 0x2F;
/// float %, selectivity depth
pub const REQ_SET_SUBHARM_DEPTH: u8 = 0xA9;
/// Reads what `REQ_SET_SUBHARM_DEPTH` writes: float %, selectivity depth
pub const REQ_GET_SUBHARM_DEPTH: u8 = 0xAA;
/// float ms, selectivity hold time
pub const REQ_SET_SUBHARM_HOLD: u8 = 0xAB;
/// Reads what `REQ_SET_SUBHARM_HOLD` writes: float ms, selectivity hold time
pub const REQ_GET_SUBHARM_HOLD: u8 = 0xAC;
/// float dBFS sub ceiling (0 = off)
pub const REQ_SET_SUBHARM_CEILING: u8 = 0xAD;
/// Reads what `REQ_SET_SUBHARM_CEILING` writes: float dBFS sub ceiling (0 = off)
pub const REQ_GET_SUBHARM_CEILING: u8 = 0xAE;

/// Control Surfaces groups, macros, display.
pub const REQ_SET_CS_GROUP: u8 = 0x20;
/// wValue = group (0-7); returns 40-byte CsGroup
pub const REQ_GET_CS_GROUP: u8 = 0x21;
/// wValue = macro (0-7), payload = 36-byte
/// CsMacroHeaderWire (name + step_count)
pub const REQ_SET_CS_MACRO: u8 = 0x22;
/// wValue = macro (0-7); returns 132-byte CsMacro
pub const REQ_GET_CS_MACRO: u8 = 0x23;
/// wValue = (step << 8) | macro, payload = 12-byte
/// CsMacroStep; all-zero record clears the step
pub const REQ_SET_CS_MACRO_STEP: u8 = 0x24;
/// GET; wValue = macro fires it, 0xFFFF cancels the
/// running one; returns 1 status byte
pub const REQ_CS_MACRO_FIRE: u8 = 0x25;
/// returns 24-byte CsExtStatusPacket
pub const REQ_GET_CS_EXT_STATUS: u8 = 0x26;
/// payload = 12-byte CsDisplayCfg
pub const REQ_SET_CS_DISPLAY_CFG: u8 = 0x27;
/// returns 16 bytes: {max_pages,
/// model_count, reserved[2]} + CsDisplayCfg
pub const REQ_GET_CS_DISPLAY_CFG: u8 = 0x28;
/// wValue = page (0-15), payload = 4-byte
/// CsDisplayPage; all-zero record clears it
pub const REQ_SET_CS_DISPLAY_PAGE: u8 = 0x29;
/// wValue = page (0-15); returns 4-byte
/// CsDisplayPage
pub const REQ_GET_CS_DISPLAY_PAGE: u8 = 0x2A;
/// returns 8-byte CsDisplayStatus
pub const REQ_GET_CS_DISPLAY_STATUS: u8 = 0x2B;

/// Psychoacoustic bass.
pub const REQ_SET_PSYBASS: u8 = 0x30;
/// returns 1 byte: psychoacoustic bass on (0/1)
pub const REQ_GET_PSYBASS: u8 = 0x31;
/// float Hz, 30..300
pub const REQ_SET_PSYBASS_CUTOFF: u8 = 0x32;
/// Reads what `REQ_SET_PSYBASS_CUTOFF` writes: float Hz, 30..300
pub const REQ_GET_PSYBASS_CUTOFF: u8 = 0x33;
/// float dB, -24..12
pub const REQ_SET_PSYBASS_HARMONICS: u8 = 0x34;
/// Reads what `REQ_SET_PSYBASS_HARMONICS` writes: float dB, -24..12
pub const REQ_GET_PSYBASS_HARMONICS: u8 = 0x35;
/// float dB, 0..18
pub const REQ_SET_PSYBASS_DRIVE: u8 = 0x36;
/// Reads what `REQ_SET_PSYBASS_DRIVE` writes: float dB, 0..18
pub const REQ_GET_PSYBASS_DRIVE: u8 = 0x37;
/// float %, 0..100
pub const REQ_SET_PSYBASS_CHARACTER: u8 = 0x38;
/// Reads what `REQ_SET_PSYBASS_CHARACTER` writes: float %, 0..100
pub const REQ_GET_PSYBASS_CHARACTER: u8 = 0x39;
/// float dB, -60..0
pub const REQ_SET_PSYBASS_ORIGINAL: u8 = 0x3A;
/// Reads what `REQ_SET_PSYBASS_ORIGINAL` writes: float dB, -60..0
pub const REQ_GET_PSYBASS_ORIGINAL: u8 = 0x3B;
/// uint16 LE output mask
pub const REQ_SET_PSYBASS_MASK: u8 = 0x3C;
/// Reads what `REQ_SET_PSYBASS_MASK` writes: uint16 LE output mask
pub const REQ_GET_PSYBASS_MASK: u8 = 0x3D;
/// Psybass parameter ids for `set_psybass_param` (host-side, not wire values).
pub const PSYBASS_PARAM_CUTOFF: u8 = 0;
/// `set_psybass_param` id: harmonics level, dB (-24..12).
pub const PSYBASS_PARAM_HARMONICS: u8 = 1;
/// `set_psybass_param` id: drive, dB (0..18).
pub const PSYBASS_PARAM_DRIVE: u8 = 2;
/// `set_psybass_param` id: character, % (0 warm .. 100 aggressive).
pub const PSYBASS_PARAM_CHARACTER: u8 = 3;
/// `set_psybass_param` id: original bass level, dB (-60..0).
pub const PSYBASS_PARAM_ORIGINAL: u8 = 4;

/// Tube modeller.
pub const REQ_SET_TUBE_PARAM: u8 = 0x3E;
/// wValue low byte = index, returns 4-byte float32 LE
pub const REQ_GET_TUBE_PARAM: u8 = 0x3F;
/// Tube parameter indices (the firmware's own; wValue of REQ_SET_TUBE_PARAM).
/// Every value travels as a float, enums and flags included.
pub const TUBE_PARAM_ENABLED: u8 = 0;
/// `set_tube_param` index: output mask (bit k = output k).
pub const TUBE_PARAM_MASK: u8 = 1;
/// `set_tube_param` index: tube type.
pub const TUBE_PARAM_TYPE: u8 = 2;
/// `set_tube_param` index: drive, dB.
pub const TUBE_PARAM_DRIVE: u8 = 3;
/// `set_tube_param` index: bias, %.
pub const TUBE_PARAM_BIAS: u8 = 4;
/// `set_tube_param` index: asymmetry, dB.
pub const TUBE_PARAM_ASYM: u8 = 5;
/// `set_tube_param` index: hardness, %.
pub const TUBE_PARAM_HARDNESS: u8 = 6;
/// `set_tube_param` index: sag, %.
pub const TUBE_PARAM_SAG: u8 = 7;
/// `set_tube_param` index: rectifier.
pub const TUBE_PARAM_RECTIFIER: u8 = 8;
/// `set_tube_param` index: output transformer on (0/1).
pub const TUBE_PARAM_XFMR: u8 = 9;
/// `set_tube_param` index: transformer damping.
pub const TUBE_PARAM_DAMPING: u8 = 10;
/// `set_tube_param` index: transformer resonance, Hz.
pub const TUBE_PARAM_RESONANCE: u8 = 11;
/// `set_tube_param` index: mix, %.
pub const TUBE_PARAM_MIX: u8 = 12;
/// `set_tube_param` index: output trim, dB.
pub const TUBE_PARAM_TRIM: u8 = 13;
/// Length of `DspState.tube`, indexed by the firmware's `TUBE_PARAM_*` (`dspi_proto::generated::tube`).
pub const TUBE_PARAM_COUNT: usize = 14;

/// Subharmonic synth parameter ids for `set_subharm_param` (host-side; each
/// field has its own request).
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

/// EQ / Preamp / Bypass / Delay.
pub const REQ_SET_EQ_PARAM: u8 = 0x42;
/// wValue = channel, band and parameter; returns float (4 bytes)
pub const REQ_GET_EQ_PARAM: u8 = 0x43;
/// Legacy: sets every input channel's preamp to the same value.
pub const REQ_SET_PREAMP: u8 = 0x44;
/// returns float dB
pub const REQ_GET_PREAMP: u8 = 0x45;
/// 1 byte 0/1, master EQ bypass
pub const REQ_SET_BYPASS: u8 = 0x46;
/// Reads what `REQ_SET_BYPASS` writes: 1 byte 0/1, master EQ bypass
pub const REQ_GET_BYPASS: u8 = 0x47;
/// wValue = wire channel, float ms
pub const REQ_SET_DELAY: u8 = 0x48;
/// Reads what `REQ_SET_DELAY` writes: wValue = wire channel, float ms
pub const REQ_GET_DELAY: u8 = 0x49;

/// Stereo upmixer.
pub const REQ_UPMIX_SET_CONFIG: u8 = 0x4A;
/// returns UpmixConfigPacket
pub const REQ_UPMIX_GET_CONFIG: u8 = 0x4B;
/// wValue = UPMIX_PARAM_*, payload = float (4 B)
pub const REQ_UPMIX_SET_PARAM: u8 = 0x4C;
/// wValue = UPMIX_PARAM_*, returns float
pub const REQ_UPMIX_GET_PARAM: u8 = 0x4D;
/// returns UpmixStatus (16 B)
pub const REQ_UPMIX_GET_STATUS: u8 = 0x4E;
/// Upmixer parameter ids (the firmware's own; wValue of REQ_UPMIX_SET_PARAM).
/// Centre mode: 0 passive, 1 adaptive, 2 off. Surround: 0 off, 1 passive, 2 adaptive.
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

/// Status / flash.
pub const REQ_GET_STATUS: u8 = 0x50;
/// GET; saves the live parameters to flash, returns a FLASH_* status byte
pub const REQ_SAVE_PARAMS: u8 = 0x51;
/// Persist the live output (IO) config in independent mode. Was the
/// deprecated REQ_LOAD_PARAMS, which is gone.
pub const REQ_SAVE_OUTPUT_CONFIG: u8 = 0x52;
/// GET; restores the factory defaults, returns a FLASH_* status byte
pub const REQ_FACTORY_RESET: u8 = 0x53;

/// Legacy channel gain/mute.
pub const REQ_SET_CHANNEL_GAIN: u8 = 0x54;
/// Vendor request: read channel gain.
pub const REQ_GET_CHANNEL_GAIN: u8 = 0x55;
/// Vendor request: write channel mute.
pub const REQ_SET_CHANNEL_MUTE: u8 = 0x56;
/// Vendor request: read channel mute.
pub const REQ_GET_CHANNEL_MUTE: u8 = 0x57;

/// Loudness.
pub const REQ_SET_LOUDNESS: u8 = 0x58;
/// returns 1 byte: loudness on (0/1)
pub const REQ_GET_LOUDNESS: u8 = 0x59;
/// float dB SPL, reference level
pub const REQ_SET_LOUDNESS_REF: u8 = 0x5A;
/// Reads what `REQ_SET_LOUDNESS_REF` writes: float dB SPL, reference level
pub const REQ_GET_LOUDNESS_REF: u8 = 0x5B;
/// float %, intensity
pub const REQ_SET_LOUDNESS_INTENSITY: u8 = 0x5C;
/// Reads what `REQ_SET_LOUDNESS_INTENSITY` writes: float %, intensity
pub const REQ_GET_LOUDNESS_INTENSITY: u8 = 0x5D;
/// payload = uint16 LE mask
pub const REQ_SET_LOUDNESS_MASK: u8 = 0xFA;
/// returns uint16 LE mask
pub const REQ_GET_LOUDNESS_MASK: u8 = 0xFB;

/// Crossfeed.
pub const REQ_SET_CROSSFEED: u8 = 0x5E;
/// returns 1 byte: crossfeed on (0/1)
pub const REQ_GET_CROSSFEED: u8 = 0x5F;
/// 1 byte preset index
pub const REQ_SET_CROSSFEED_PRESET: u8 = 0x60;
/// Reads what `REQ_SET_CROSSFEED_PRESET` writes: 1 byte preset index
pub const REQ_GET_CROSSFEED_PRESET: u8 = 0x61;
/// float Hz, custom frequency
pub const REQ_SET_CROSSFEED_FREQ: u8 = 0x62;
/// Reads what `REQ_SET_CROSSFEED_FREQ` writes: float Hz, custom frequency
pub const REQ_GET_CROSSFEED_FREQ: u8 = 0x63;
/// float dB, custom feed level
pub const REQ_SET_CROSSFEED_FEED: u8 = 0x64;
/// Reads what `REQ_SET_CROSSFEED_FEED` writes: float dB, custom feed level
pub const REQ_GET_CROSSFEED_FEED: u8 = 0x65;
/// 1 byte 0/1, interaural time delay
pub const REQ_SET_CROSSFEED_ITD: u8 = 0x66;
/// Reads what `REQ_SET_CROSSFEED_ITD` writes: 1 byte 0/1, interaural time delay
pub const REQ_GET_CROSSFEED_ITD: u8 = 0x67;
/// payload = uint8 pair mask
pub const REQ_SET_CROSSFEED_OUTPUTS: u8 = 0xFC;
/// returns uint8 pair mask
pub const REQ_GET_CROSSFEED_OUTPUTS: u8 = 0xFD;

/// ADAT input (RP2350).
pub const REQ_SET_ADAT_INPUT_ENABLE: u8 = 0x68;
/// returns 1 byte: ADAT input enabled (0/1)
pub const REQ_GET_ADAT_INPUT_ENABLE: u8 = 0x69;
/// wValue = GPIO; PIN_RESET_TO_DEFAULT
/// clears to unset (only while disabled)
pub const REQ_SET_ADAT_INPUT_PIN: u8 = 0x6A;
/// returns uint8_t (0xFF = unset)
pub const REQ_GET_ADAT_INPUT_PIN: u8 = 0x6B;
/// GET; wValue = mode (0 master, 1 slave), returns a status byte
pub const REQ_SET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6C;
/// Returns the setting `REQ_SET_ADAT_INPUT_CLOCK_MODE` writes.
pub const REQ_GET_ADAT_INPUT_CLOCK_MODE: u8 = 0x6D;
/// returns the 20-byte ADAT input status
pub const REQ_GET_ADAT_INPUT_STATUS: u8 = 0x6E;

/// Matrix mixer / outputs.
pub const REQ_SET_MATRIX_ROUTE: u8 = 0x70;
/// reads one matrix crosspoint
pub const REQ_GET_MATRIX_ROUTE: u8 = 0x71;
/// wValue = output, 1 byte 0/1
pub const REQ_SET_OUTPUT_ENABLE: u8 = 0x72;
/// Reads what `REQ_SET_OUTPUT_ENABLE` writes: wValue = output, 1 byte 0/1
pub const REQ_GET_OUTPUT_ENABLE: u8 = 0x73;
/// wValue = output, float dB
pub const REQ_SET_OUTPUT_GAIN: u8 = 0x74;
/// Reads what `REQ_SET_OUTPUT_GAIN` writes: wValue = output, float dB
pub const REQ_GET_OUTPUT_GAIN: u8 = 0x75;
/// wValue = output, 1 byte 0/1
pub const REQ_SET_OUTPUT_MUTE: u8 = 0x76;
/// Reads what `REQ_SET_OUTPUT_MUTE` writes: wValue = output, 1 byte 0/1
pub const REQ_GET_OUTPUT_MUTE: u8 = 0x77;
/// wValue = output, float ms
pub const REQ_SET_OUTPUT_DELAY: u8 = 0x78;
/// Reads what `REQ_SET_OUTPUT_DELAY` writes: wValue = output, float ms
pub const REQ_GET_OUTPUT_DELAY: u8 = 0x79;

/// Core 1.
pub const REQ_GET_CORE1_MODE: u8 = 0x7A;
/// wValue = output; returns 1 byte, non-zero when enabling it conflicts with core 1's mode
pub const REQ_GET_CORE1_CONFLICT: u8 = 0x7B;

/// Pins.
pub const REQ_SET_OUTPUT_PIN: u8 = 0x7C;
/// Reads what `REQ_SET_OUTPUT_PIN` writes: wValue = (GPIO<<8)|output_index;
/// GPIO = PIN_RESET_TO_DEFAULT restores the default
pub const REQ_GET_OUTPUT_PIN: u8 = 0x7D;

/// Identification.
pub const REQ_GET_SERIAL: u8 = 0x7E;
/// returns the platform and the firmware version
pub const REQ_GET_PLATFORM: u8 = 0x7F;
/// 64-byte git-describe/date blob; provenance for
/// humans only, no software may gate on it
pub const REQ_GET_BUILD_INFO: u8 = 0x80;

/// Output limiter (sub-command multiplexed).
pub const REQ_LIMITER: u8 = 0x81;
/// REQ_LIMITER wValue low byte: parameter index (high byte = output).
pub const LIMITER_PARAM_ENABLED: u8 = 0;
/// `set_limiter_param` parameter: threshold, dB.
pub const LIMITER_PARAM_THRESHOLD_DB: u8 = 1;
/// `set_limiter_param` parameter: release, ms.
pub const LIMITER_PARAM_RELEASE_MS: u8 = 2;
/// `set_limiter_param` parameter: link group (0 = none).
pub const LIMITER_PARAM_LINK_GROUP: u8 = 3;
/// GET: per-output gain reduction, u16 in 0.01 dB.
pub const LIMITER_GET_METER: u8 = 0x80;
/// GET: engaged, lookahead, block, num_outputs.
pub const LIMITER_GET_STATUS: u8 = 0x81;
/// SET output byte: apply to every output.
pub const LIMITER_ALL_OUTPUTS: u8 = 0xFF;

/// Clip detection.
pub const REQ_CLEAR_CLIPS: u8 = 0x83;

/// Control Surfaces bindings / IR.
pub const REQ_SET_CS_BINDING: u8 = 0x84;
/// wValue = slot, returns 24-byte CsBinding
pub const REQ_GET_CS_BINDING: u8 = 0x85;
/// wValue = 0xFFFF: header + type table; wValue = noun: 12-byte noun descriptor
pub const REQ_GET_CS_CAPS: u8 = 0x86;
/// returns 41-byte CsStatusPacket
pub const REQ_GET_CS_STATUS: u8 = 0x87;
/// wValue = slot (0-15), payload = 1-32 byte name (a single
/// NUL byte clears it); apply-live-only preview, deferred,
/// result via REQ_GET_CS_STATUS
pub const REQ_SET_CS_NAME: u8 = 0x8B;
/// wValue = slot, returns 32-byte NUL-terminated live name
pub const REQ_GET_CS_NAME: u8 = 0x8C;
/// wValue = sub-slot (0-15), payload = 16-byte IrCommand;
/// apply-live-only preview, deferred to the main loop
pub const REQ_SET_CS_IR_CMD: u8 = 0x8D;
/// wValue = sub-slot, returns 16-byte IrCommand
pub const REQ_GET_CS_IR_CMD: u8 = 0x8E;
/// GET; wValue 1 = arm, 0 = cancel (each returns 1 status
/// byte), 2 = read result (returns 8-byte
/// {state, protocol, 0, 0, code_le32})
pub const REQ_CS_IR_LEARN: u8 = 0x8F;
/// GET; persists the whole live CS config in one flash
/// write; deferred, result via REQ_GET_CS_STATUS
pub const REQ_CS_SAVE: u8 = 0x9D;
/// GET; discards the live preview and re-applies the
/// stored config; deferred, result via REQ_GET_CS_STATUS
pub const REQ_CS_REVERT: u8 = 0x9E;

/// I2S clock mode.
pub const REQ_SET_I2S_CLOCK_MODE: u8 = 0x88;
/// returns uint8_t live mode (pending change not reflected)
pub const REQ_GET_I2S_CLOCK_MODE: u8 = 0x89;
/// returns 16-byte I2sSlaveStatusPacket
pub const REQ_GET_I2S_SLAVE_STATUS: u8 = 0x8A;
/// wValue = 0 unified / 1 split; returns PIN_CONFIG_* status
pub const REQ_SET_I2S_CLOCK_PIN_MODE: u8 = 0xFE;
/// returns uint8_t live mode
pub const REQ_GET_I2S_CLOCK_PIN_MODE: u8 = 0xFF;

/// Presets.
pub const REQ_PRESET_SAVE: u8 = 0x90;
/// GET; wValue = slot, returns a PRESET_* status byte
pub const REQ_PRESET_LOAD: u8 = 0x91;
/// GET; wValue = slot, returns a PRESET_* status byte
pub const REQ_PRESET_DELETE: u8 = 0x92;
/// wValue = slot; returns the name, at most 32 bytes
pub const REQ_PRESET_GET_NAME: u8 = 0x93;
/// wValue = slot, payload = NUL-terminated name
pub const REQ_PRESET_SET_NAME: u8 = 0x94;
/// returns the preset directory, 7 bytes (5 from older firmware)
pub const REQ_PRESET_GET_DIR: u8 = 0x95;
/// payload = [mode, default_slot]
pub const REQ_PRESET_SET_STARTUP: u8 = 0x96;
/// Returns what `REQ_PRESET_SET_STARTUP` writes.
pub const REQ_PRESET_GET_STARTUP: u8 = 0x97;
/// Replaced the old include-pins flag (same 0/1 meaning).
pub const REQ_SET_OUTPUT_CONFIG_MODE: u8 = 0x98;
/// returns uint8_t mode
pub const REQ_GET_OUTPUT_CONFIG_MODE: u8 = 0x99;
/// returns 1 byte: the active slot
pub const REQ_PRESET_GET_ACTIVE: u8 = 0x9A;

/// Channel names.
pub const REQ_SET_CHANNEL_NAME: u8 = 0x9B;
/// wValue = wire channel; returns the name, at most 32 bytes
pub const REQ_GET_CHANNEL_NAME: u8 = 0x9C;

/// Bulk parameter transfer.
pub const REQ_GET_ALL_PARAMS: u8 = 0xA0;
/// payload = the whole bulk image in one transfer
pub const REQ_SET_ALL_PARAMS: u8 = 0xA1;
/// Chunked bulk-params access (USB-only).  Works around the Windows/WinUSB
/// 4 KB control-transfer cap that the full WireBulkParams (5864 B at V16)
/// cannot cross (GitHub issue #62).  wValue = byte offset, wLength = chunk
/// size.  GET: offset 0 snapshots the struct under the bulk lock, later
/// offsets read the snapshot out; the lock frees after the final chunk.
/// SET: chunks must be sequential from offset 0; the normal deferred apply
/// fires when the last byte lands.  UART and I2C have no size cap and use
/// plain 0xA0/0xA1; these two commands are refused on those transports.
pub const REQ_GET_ALL_PARAMS_CHUNK: u8 = 0xA2;
/// Chunked bulk-params access (USB-only).  Works around the Windows/WinUSB
/// 4 KB control-transfer cap that the full WireBulkParams (5864 B at V16)
/// cannot cross (GitHub issue #62).  wValue = byte offset, wLength = chunk
/// size.  GET: offset 0 snapshots the struct under the bulk lock, later
/// offsets read the snapshot out; the lock frees after the final chunk.
/// SET: chunks must be sequential from offset 0; the normal deferred apply
/// fires when the last byte lands.  UART and I2C have no size cap and use
/// plain 0xA0/0xA1; these two commands are refused on those transports.
pub const REQ_SET_ALL_PARAMS_CHUNK: u8 = 0xA3;

/// Signal generator.
pub const REQ_SIGGEN_SET_CONFIG: u8 = 0xA4;
/// returns SiggenConfig
pub const REQ_SIGGEN_GET_CONFIG: u8 = 0xA5;
/// wValue = SIGGEN_CTL_*; returns status byte
pub const REQ_SIGGEN_CONTROL: u8 = 0xA6;
/// returns SiggenStatus (16 B)
pub const REQ_SIGGEN_GET_STATUS: u8 = 0xA7;
/// wValue = type index / 0xFFFF = header
pub const REQ_SIGGEN_GET_CAPS: u8 = 0xA8;

/// Buffer / USB statistics.
pub const REQ_GET_BUFFER_STATS: u8 = 0xB0;
/// GET; wValue 1 restarts the watermarks, returns 1 byte (1 = ACK)
pub const REQ_RESET_BUFFER_STATS: u8 = 0xB1;
/// Vendor request: read usb error stats.
pub const REQ_GET_USB_ERROR_STATS: u8 = 0xB2;
/// Vendor request: reset the USB error counters.
pub const REQ_RESET_USB_ERROR_STATS: u8 = 0xB3;

/// Volume leveller.
pub const REQ_SET_LEVELLER_ENABLE: u8 = 0xB4;
/// returns 1 byte: leveller on (0/1)
pub const REQ_GET_LEVELLER_ENABLE: u8 = 0xB5;
/// float %
pub const REQ_SET_LEVELLER_AMOUNT: u8 = 0xB6;
/// Reads what `REQ_SET_LEVELLER_AMOUNT` writes: float %
pub const REQ_GET_LEVELLER_AMOUNT: u8 = 0xB7;
/// 1 byte: 0 slow, 1 medium, 2 fast
pub const REQ_SET_LEVELLER_SPEED: u8 = 0xB8;
/// Reads what `REQ_SET_LEVELLER_SPEED` writes: 1 byte: 0 slow, 1 medium, 2 fast
pub const REQ_GET_LEVELLER_SPEED: u8 = 0xB9;
/// float dB
pub const REQ_SET_LEVELLER_MAX_GAIN: u8 = 0xBA;
/// Reads what `REQ_SET_LEVELLER_MAX_GAIN` writes: float dB
pub const REQ_GET_LEVELLER_MAX_GAIN: u8 = 0xBB;
/// 1 byte 0/1
pub const REQ_SET_LEVELLER_LOOKAHEAD: u8 = 0xBC;
/// Reads what `REQ_SET_LEVELLER_LOOKAHEAD` writes: 1 byte 0/1
pub const REQ_GET_LEVELLER_LOOKAHEAD: u8 = 0xBD;
/// float dB
pub const REQ_SET_LEVELLER_GATE: u8 = 0xBE;
/// Reads what `REQ_SET_LEVELLER_GATE` writes: float dB
pub const REQ_GET_LEVELLER_GATE: u8 = 0xBF;
/// 2 bytes: [detector_mask, apply_mask]
pub const REQ_SET_LEVELLER_MASKS: u8 = 0xDE;
/// Reads what `REQ_SET_LEVELLER_MASKS` writes: 2 bytes: [detector_mask, apply_mask]
pub const REQ_GET_LEVELLER_MASKS: u8 = 0xDF;

/// Output types / I2S / MCK.
pub const REQ_SET_OUTPUT_TYPE: u8 = 0xC0;
/// wValue = output slot; returns 1 byte OUTPUT_TYPE_*
pub const REQ_GET_OUTPUT_TYPE: u8 = 0xC1;
/// wValue = (role<<8)|GPIO; GPIO =
/// PIN_RESET_TO_DEFAULT restores the role's default
pub const REQ_SET_I2S_BCK_PIN: u8 = 0xC2;
/// Reads what `REQ_SET_I2S_BCK_PIN` writes: wValue = (role<<8)|GPIO; GPIO =
/// PIN_RESET_TO_DEFAULT restores the role's default
pub const REQ_GET_I2S_BCK_PIN: u8 = 0xC3;
/// GET; wValue = 0/1, returns a status byte
pub const REQ_SET_MCK_ENABLE: u8 = 0xC4;
/// Returns the setting `REQ_SET_MCK_ENABLE` writes.
pub const REQ_GET_MCK_ENABLE: u8 = 0xC5;
/// wValue = GPIO; PIN_RESET_TO_DEFAULT
/// restores PICO_I2S_MCK_PIN
pub const REQ_SET_MCK_PIN: u8 = 0xC6;
/// Reads what `REQ_SET_MCK_PIN` writes: wValue = GPIO; PIN_RESET_TO_DEFAULT
/// restores PICO_I2S_MCK_PIN
pub const REQ_GET_MCK_PIN: u8 = 0xC7;
/// GET; wValue = 128 or 256, returns a status byte
pub const REQ_SET_MCK_MULTIPLIER: u8 = 0xC8;
/// Returns the setting `REQ_SET_MCK_MULTIPLIER` writes.
pub const REQ_GET_MCK_MULTIPLIER: u8 = 0xC9;

/// ADAT output (RP2350).
pub const REQ_SET_ADAT_ENABLE: u8 = 0xCA;
/// returns uint8_t configured enable
pub const REQ_GET_ADAT_ENABLE: u8 = 0xCB;
/// wValue = GPIO, returns status byte; GPIO =
/// PIN_RESET_TO_DEFAULT restores PICO_ADAT_PIN
pub const REQ_SET_ADAT_PIN: u8 = 0xCC;
/// returns uint8_t
pub const REQ_GET_ADAT_PIN: u8 = 0xCD;
/// returns AdatStatus (8 B)
pub const REQ_GET_ADAT_STATUS: u8 = 0xCE;

/// Per-input preamp.
pub const REQ_SET_PREAMP_CH: u8 = 0xD0;
/// wValue = channel index, returns float dB
pub const REQ_GET_PREAMP_CH: u8 = 0xD1;

/// Master volume.
pub const REQ_SET_MASTER_VOLUME: u8 = 0xD2;
/// returns float dB
pub const REQ_GET_MASTER_VOLUME: u8 = 0xD3;
/// payload = uint8_t mode (see MASTER_VOLUME_MODE_*)
pub const REQ_SET_MASTER_VOLUME_MODE: u8 = 0xD4;
/// returns uint8_t mode
pub const REQ_GET_MASTER_VOLUME_MODE: u8 = 0xD5;
/// no payload, stores live master vol to directory
pub const REQ_SAVE_MASTER_VOLUME: u8 = 0xD6;
/// returns float dB from directory's independent field
pub const REQ_GET_SAVED_MASTER_VOLUME: u8 = 0xD7;

/// Per-band bypass.
pub const REQ_SET_BAND_BYPASS: u8 = 0xD8;
/// Per-Band Bypass Commands
/// wValue = (channel << 8) | band; payload = uint8_t (1 = bypass, anything else = active)
pub const REQ_GET_BAND_BYPASS: u8 = 0xD9;

/// User volume / mute.
pub const REQ_SET_USER_VOLUME: u8 = 0xDA;
/// returns float dB (audio_state.volume / 256)
pub const REQ_GET_USER_VOLUME: u8 = 0xDB;
/// payload = uint8_t (0/1)
pub const REQ_SET_USER_MUTE: u8 = 0xDC;
/// returns uint8_t (0/1)
pub const REQ_GET_USER_MUTE: u8 = 0xDD;

/// Input source / S/PDIF / I2S input.
pub const REQ_SET_INPUT_SOURCE: u8 = 0xE0;
/// returns uint8_t
pub const REQ_GET_INPUT_SOURCE: u8 = 0xE1;
/// returns 16-byte status struct (Phase 2)
pub const REQ_GET_SPDIF_RX_STATUS: u8 = 0xE2;
/// returns 24-byte IEC 60958 channel status (Phase 2)
pub const REQ_GET_SPDIF_RX_CH_STATUS: u8 = 0xE3;
/// wValue = (index<<8)|GPIO; index 0..3 selects the
/// SPDIF input (old hosts send wValue=pin => index 0).
/// GPIO = PIN_RESET_TO_DEFAULT restores that input's
/// default. Returns status byte.
pub const REQ_SET_SPDIF_RX_PIN: u8 = 0xE4;
/// wValue = index (0..3); returns that input's GPIO
pub const REQ_GET_SPDIF_RX_PIN: u8 = 0xE5;
/// wValue = (index<<8)|enable; index 1..3, enable 0/1.
/// Enables/disables an optional SPDIF input. Returns status byte.
pub const REQ_SET_SPDIF_INPUT_ENABLE: u8 = 0xE9;
/// returns 6 bytes: count, enable mask (bit0=input1),
/// then the GPIO for inputs 0..3
pub const REQ_GET_SPDIF_INPUT_CONFIG: u8 = 0xEF;
/// payload = uint32_t Hz (44100/48000/96000)
pub const REQ_SET_INPUT_RATE: u8 = 0xED;
/// returns 2x uint32_t {current Hz, selected I2S Hz}
pub const REQ_GET_INPUT_RATE: u8 = 0xEE;
/// wValue = (pair<<8)|GPIO; pair 0..3 selects the
/// stereo pair (old hosts send wValue=pin => pair 0).
/// GPIO = PIN_RESET_TO_DEFAULT restores that pair's
/// default. Returns status byte.
pub const REQ_SET_I2S_RX_PIN: u8 = 0xF1;
/// wValue = pair (0..3); returns that pair's GPIO
pub const REQ_GET_I2S_RX_PIN: u8 = 0xF2;
/// wValue = channel count (2/4/6/8); returns status byte
pub const REQ_SET_I2S_INPUT_CHANNELS: u8 = 0xF3;
/// returns uint8_t (active count)
pub const REQ_GET_I2S_INPUT_CHANNELS: u8 = 0xF4;

/// LG Sound Sync.
pub const REQ_SET_LG_SOUND_SYNC_ENABLE: u8 = 0xE6;
/// returns uint8_t
pub const REQ_GET_LG_SOUND_SYNC_ENABLE: u8 = 0xE7;
/// returns 16-byte LgSoundSyncStatus
pub const REQ_GET_LG_SOUND_SYNC_STATUS: u8 = 0xE8;

/// DAC hardware mute.
pub const REQ_SET_DAC_HW_MUTE_CONFIG: u8 = 0xEA;
/// returns 16-byte DacHwMuteConfig
pub const REQ_GET_DAC_HW_MUTE_CONFIG: u8 = 0xEB;
/// no payload; pulses mute ~1s
pub const REQ_TEST_DAC_HW_MUTE: u8 = 0xEC;

/// System.
pub const REQ_ENTER_BOOTLOADER: u8 = 0xF0;

/// UART / I2C control interfaces.
pub const REQ_SET_UART_CONFIG: u8 = 0xF5;
/// returns UartCtrlConfig (live)
pub const REQ_GET_UART_CONFIG: u8 = 0xF6;
/// payload = I2cCtrlConfig (8 bytes); USB-only
pub const REQ_SET_I2C_CONFIG: u8 = 0xF7;
/// returns I2cCtrlConfig (live)
pub const REQ_GET_I2C_CONFIG: u8 = 0xF8;
/// returns CtrlIfaceStatus (8 bytes)
pub const REQ_GET_CTRL_IFACE_STATUS: u8 = 0xF9;

/// Status codes.
pub const PIN_CONFIG_SUCCESS: u8 = 0x00;
/// Pin configuration status: the GPIO cannot do this.
pub const PIN_CONFIG_INVALID_PIN: u8 = 0x01;
/// Pin configuration status: another function holds the GPIO.
pub const PIN_CONFIG_PIN_IN_USE: u8 = 0x02;
/// Pin configuration status: no such output slot.
pub const PIN_CONFIG_INVALID_OUTPUT: u8 = 0x03;
/// Pin configuration status: the output is running; stop it first.
pub const PIN_CONFIG_OUTPUT_ACTIVE: u8 = 0x04;
/// non-pin field out of range (baud, I2C address).
pub const PIN_CONFIG_INVALID_PARAM: u8 = 0x05;
/// Sent as the pin byte of any single-pin SET to restore the platform default.
pub const PIN_RESET_TO_DEFAULT: u8 = 0xFF;
/// Preset status: done.
pub const PRESET_OK: u8 = 0x00;
/// Preset status: no such slot.
pub const PRESET_ERR_INVALID_SLOT: u8 = 0x01;
/// Preset status: the slot holds no preset.
pub const PRESET_ERR_SLOT_EMPTY: u8 = 0x02;
/// Preset status: the stored preset failed its CRC.
pub const PRESET_ERR_CRC: u8 = 0x03;
/// Preset status: the flash write failed.
pub const PRESET_ERR_FLASH_WRITE: u8 = 0x04;
/// `save_params` / `load_params` / `factory_reset` status: done. These `FLASH_*` codes are the old core's; the firmware headers do not name them.
pub const FLASH_OK: u8 = 0;
/// Flash status: the write failed (also any refused revert-to-saved).
pub const FLASH_ERR_WRITE: u8 = 1;
/// Flash status: nothing saved to load.
pub const FLASH_ERR_NO_DATA: u8 = 2;
/// Flash status: the stored data failed its CRC.
pub const FLASH_ERR_CRC: u8 = 3;

/// wIndex conventions (as used by the macOS Console).
///
/// wIndex for EQ, preamp, bypass, delay, status, loudness, crossfeed, volume, core1.
pub const WINDEX_GLOBAL: u16 = 0;
/// wIndex for matrix/output/pin/preset/channel-name/bulk commands.
pub const WINDEX_OUTPUT: u16 = 2;

/// Firmware the core is built against.
///
/// Wire format this core reads and writes.
pub const WIRE_FORMAT_VERSION: u8 = 32;
/// Oldest wire format with the unified channel model. Anything older is
/// incompatible (V16 broke compatibility with no migration).
pub const MIN_WIRE_FORMAT_VERSION: u8 = 16;

/// Bulk parameter wire format (WireBulkParams, V32).
///
/// sizeof(WireBulkParams) at V32.
pub const BULK_PARAMS_SIZE: usize = 6136;
/// Chunk size for REQ_GET/SET_ALL_PARAMS_CHUNK. Kept well under the 4 KB
/// control-transfer limit (Linux usbfs caps a control transfer at one page).
pub const BULK_CHUNK_SIZE: usize = 2048;
/// Byte offset of the header in the bulk image.
pub const OFF_HEADER: usize = 0;
/// Byte offset of the global parameters in the bulk image.
pub const OFF_GLOBAL: usize = 16;
/// Byte offset of the crossfeed parameters in the bulk image.
pub const OFF_CROSSFEED: usize = 32;
/// Byte offset of the legacy section in the bulk image.
pub const OFF_LEGACY: usize = 48;
/// Byte offset of the channel delays in the bulk image.
pub const OFF_DELAYS: usize = 64;
/// Byte offset of the matrix crosspoints in the bulk image.
pub const OFF_CROSSPOINTS: usize = 132;
/// Byte offset of the output channels in the bulk image.
pub const OFF_OUTPUTS: usize = 708;
/// Byte offset of the pin configuration in the bulk image.
pub const OFF_PINS: usize = 816;
/// Byte offset of the EQ bands in the bulk image.
pub const OFF_EQ: usize = 824;
/// Byte offset of the channel names in the bulk image.
pub const OFF_CHANNEL_NAMES: usize = 4088;
/// Byte offset of the I2S configuration in the bulk image.
pub const OFF_I2S: usize = 4632;
/// Byte offset of the leveller in the bulk image.
pub const OFF_LEVELLER: usize = 4648;
/// Byte offset of the input preamps in the bulk image.
pub const OFF_PREAMP: usize = 4668;
/// Byte offset of the master volume in the bulk image.
pub const OFF_MASTER_VOLUME: usize = 4700;
/// Byte offset of the input configuration in the bulk image.
pub const OFF_INPUT_CONFIG: usize = 4716;
/// Byte offset of LG Sound Sync in the bulk image.
pub const OFF_LG_SOUND_SYNC: usize = 4732;
/// Byte offset of the user volume in the bulk image.
pub const OFF_USER_VOLUME: usize = 4748;
/// Byte offset of the DAC hardware mute in the bulk image.
pub const OFF_DAC_HW_MUTE: usize = 4764;
/// Byte offset of the crossover bands in the bulk image.
pub const OFF_CROSSOVER: usize = 4780;
/// Byte offset of the ADAT output configuration in the bulk image.
pub const OFF_ADAT: usize = 5868;
/// Byte offset of the psychoacoustic bass in the bulk image.
pub const OFF_PSYBASS: usize = 5876;
/// Byte offset of the upmixer in the bulk image.
pub const OFF_UPMIX: usize = 5900;
/// Byte offset of the subharmonic synthesizer in the bulk image.
pub const OFF_SUBHARM: usize = 5944;
/// Byte offset of the tube preamp in the bulk image.
pub const OFF_TUBE: usize = 5980;
/// Byte offset of the output limiter in the bulk image.
pub const OFF_LIMITER: usize = 6028;

/// Packets.
///
/// `fw_beta` for a beta that can't say which one it is: 1.1.6 betas 1 and 2
/// predate the ordinal and answer as plain 1.1.6. Sorts below every beta.
pub const FW_BETA_EARLY: u8 = 0xFF;

// ── rta.rs ─────────────────────────────────────────────────────────

/// Spectrum analyser (RTA): protocol version, record array sizes, taps and the
/// level floor.
pub const RTA_VERSION: u8 = 3;
/// Spectrum analyser: band slots per channel (`RtaCapsInfo::centres`, `RtaChannelFrame::avg_db` / `peak_db`), `RTA_MAX_BANDS` (rta_fft.h:42).
pub const RTA_MAX_BANDS: usize = 37;
/// Channels at one tap the snapshot holds (the mask is 16 bits).
pub const RTA_MAX_CHANNELS: usize = 16;
/// Spectrum analyser: bin slots in `RtaSnapshot::bins_db` (N/2 at the largest transform, 1024 points).
pub const RTA_MAX_BINS: usize = 512;
/// Spectrum analyser tap: after the per-input PEQ, before the matrix.
pub const RTA_TAP_INPUT: u8 = 0;
/// Spectrum analyser tap: after output gain and delay.
pub const RTA_TAP_OUTPUT: u8 = 1;
/// Level byte of the band and bin frames at the floor: -121.5 dBFS.
pub const RTA_FLOOR_DB: f32 = -121.5;

// ── siggen.rs ──────────────────────────────────────────────────────

/// Signal generator: protocol version and the number of signal types.
pub const SIGGEN_VERSION: u8 = 1;
/// Length of `SiggenCaps.types` (the old core's fixed table size).
pub const SIGGEN_TYPE_COUNT: usize = 15;

/// Signal generator: `SiggenConfig.flags` bits and CONTROL actions.
pub const SIGGEN_FLAG_RAW: u8 = 0x01;
/// `SiggenConfig.flags`: noise gets an independent generator per channel.
pub const SIGGEN_FLAG_DECORR: u8 = 0x02;
/// `SiggenConfig.flags`: play the masked outputs one at a time.
pub const SIGGEN_FLAG_WALK: u8 = 0x04;
/// CONTROL actions.
pub const SIGGEN_CTL_STOP: u16 = 0;
/// `siggen_control` action: (re)start with the staged config.
pub const SIGGEN_CTL_START: u16 = 1;
/// `siggen_control` action: stop at once, no fade.
pub const SIGGEN_CTL_STOP_NOW: u16 = 2;

// ── stats.rs ───────────────────────────────────────────────────────

/// REQ_GET_STATUS sub-queries that return one u32 (cumulative since boot).
pub const STAT_PDM_RING_OVERRUNS: u16 = 3;
/// `fetch_stat`: PDM ring underruns since boot.
pub const STAT_PDM_RING_UNDERRUNS: u16 = 4;
/// `fetch_stat`: PDM DMA overruns since boot.
pub const STAT_PDM_DMA_OVERRUNS: u16 = 5;
/// `fetch_stat`: PDM DMA underruns since boot.
pub const STAT_PDM_DMA_UNDERRUNS: u16 = 6;
/// `fetch_stat`: retired (S/PDIF overruns); current firmware answers 0.
pub const STAT_SPDIF_OVERRUNS: u16 = 7;
/// `fetch_stat`: S/PDIF underruns since boot.
pub const STAT_SPDIF_UNDERRUNS: u16 = 8;
/// `fetch_stat`: system clock, Hz.
pub const STAT_CLOCK_HZ: u16 = 13;
/// `fetch_stat`: core voltage, mV.
pub const STAT_CORE_MV: u16 = 14;
/// `fetch_stat`: audio sample rate, Hz.
pub const STAT_SAMPLE_RATE: u16 = 15;
/// Centi-degrees C, signed (an i16 widened to u32).
pub const STAT_TEMPERATURE: u16 = 16;
/// S/PDIF DMA starvations: total, then instances 0-3 (18-21).
pub const STAT_STARVATION_TOTAL: u16 = 17;
/// `fetch_stat`: S/PDIF DMA starvations of instance 0; instances 1-3 follow (18-21).
pub const STAT_STARVATION_FIRST: u16 = 18;
/// `fetch_stat`: USB audio ring overruns since boot.
pub const STAT_USB_RING_OVERRUNS: u16 = 22;

// ── presetfile.rs ──────────────────────────────────────────────────

/// Preset files (`.dspipreset`): the schema version.
pub const SCHEMA_VERSION: i64 = 1;

/// Preset import: what is applied besides the DSP settings (bits).
pub const IMPORT_VOLUMES: u32 = 1;
/// `config_import` option: also apply the hardware I/O and the limiters.
pub const IMPORT_HARDWARE: u32 = 2;

// ── filterfile.rs ──────────────────────────────────────────────────

/// Filter files: the format version.
pub const FORMAT_VERSION: u32 = 2;

// ── cs.rs ──────────────────────────────────────────────────────────

/// Control surfaces: table sizes of the binding, IR, group, macro, step and
/// page records.
pub const CS_MAX_BINDINGS: usize = 16;
/// Control surfaces: IR command slots.
pub const CS_MAX_IR: usize = 16;
/// Control surfaces: groups.
pub const CS_MAX_GROUPS: usize = 8;
/// Control surfaces: macros.
pub const CS_MAX_MACROS: usize = 8;
/// Control surfaces: steps per macro.
pub const CS_MAX_STEPS: usize = 8;
/// Control surfaces: display pages.
pub const CS_MAX_PAGES: usize = 16;

// ── state.rs ───────────────────────────────────────────────────────

/// `DspState.compat`: the firmware compatibility judged at connect.
///
/// Firmware compatibility of the connected device, as judged at connect time.
pub const COMPAT_UNKNOWN: u8 = 0;
/// Wire format and firmware match this core.
pub const COMPAT_OK: u8 = 1;
/// Firmware is too old (pre-V32 wire format, or the old USB vendor ID).
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;
/// Firmware is newer than this core understands.
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

// -- WP QD6 --
/// `dspi_wire_features`: psychoacoustic bass (V23).
pub const WIRE_FEAT_PSYBASS: u32 = 1 << 6;
/// `dspi_wire_features`: stereo upmixer (V25).
pub const WIRE_FEAT_UPMIX: u32 = 1 << 8;
/// `dspi_wire_features`: subharmonic synthesizer (V29).
pub const WIRE_FEAT_SUBHARM: u32 = 1 << 11;
/// `dspi_wire_features`: tube modeller (V31).
pub const WIRE_FEAT_TUBE: u32 = 1 << 13;
/// `dspi_wire_features`: output limiter (V32).
pub const WIRE_FEAT_LIMITER: u32 = 1 << 14;

// ── types.rs ───────────────────────────────────────────────────────

/// Filter types (firmware config.h `enum FilterType`). Value space: 0..13 PEQ
/// types, 14..31 reserved, 32..63 crossover types (only valid in crossover
/// bands), 64+ reserved.
pub const FILTER_FLAT: u8 = 0;
/// Filter type: peaking.
pub const FILTER_PEAKING: u8 = 1;
/// Filter type: low shelf.
pub const FILTER_LOWSHELF: u8 = 2;
/// Filter type: high shelf.
pub const FILTER_HIGHSHELF: u8 = 3;
/// Filter type: low-pass.
pub const FILTER_LOWPASS: u8 = 4;
/// Filter type: high-pass.
pub const FILTER_HIGHPASS: u8 = 5;
/// Filter type: notch.
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
/// Filter type: the last crossover type.
pub const FILTER_XOVER_LAST: u8 = 63;

/// Default Q (Butterworth), also the Linkwitz Transform target-Q default.
pub const DEFAULT_Q: f32 = 0.707;

/// Array sizes and limits of the records (RP2350 maxima).
///
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
