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
pub const MAX_PRESETS: usize = dspi_proto::generated::status::PRESET_SLOTS as usize;
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
/// `set_psybass_param` id: speaker cutoff, Hz (30..300). Host-side ids, the
/// old core's (each parameter has its own request on the wire).
pub const PSYBASS_PARAM_CUTOFF: u8 = 0;
/// `set_psybass_param` id: harmonics level, dB (-24..12).
pub const PSYBASS_PARAM_HARMONICS: u8 = 1;
/// `set_psybass_param` id: drive, dB (0..18).
pub const PSYBASS_PARAM_DRIVE: u8 = 2;
/// `set_psybass_param` id: character, % (0 warm .. 100 aggressive).
pub const PSYBASS_PARAM_CHARACTER: u8 = 3;
/// `set_psybass_param` id: original bass level, dB (-60..0).
pub const PSYBASS_PARAM_ORIGINAL: u8 = 4;
/// Number of `PSYBASS_PARAM_*` ids.
pub const PSYBASS_PARAM_COUNT: usize = 5;
// -- WP C5 --
/// `save_params` / `load_params` / `factory_reset` status: done. These
/// `FLASH_*` codes are the old core's; the firmware headers do not name
/// them.
pub const FLASH_OK: u8 = 0;
/// Flash status: the write failed (also any refused revert-to-saved).
pub const FLASH_ERR_WRITE: u8 = 1;
/// Flash status: nothing saved to load.
pub const FLASH_ERR_NO_DATA: u8 = 2;
/// Flash status: the stored data failed its CRC.
pub const FLASH_ERR_CRC: u8 = 3;
// -- WP C6 --
// -- WP C7 --
/// `NotifyResult.flags`: some parameter changed; re-read the state.
pub const NOTIFY_STATE: u32 = 1 << 0;
/// A change that moves the graph (filters, gains, bypass, output enable).
pub const NOTIFY_CURVES: u32 = 1 << 1;
/// Everything was re-read from the device (preset load, a foreign bulk
/// write, missed packets).
pub const NOTIFY_REFRESHED: u32 = 1 << 2;
/// The active USB input channel count changed.
pub const NOTIFY_INPUT_FORMAT: u32 = 1 << 3;
/// A preset was loaded on the device (`DspState.active_preset_slot`).
pub const NOTIFY_PRESET: u32 = 1 << 4;
/// The signal generator started or stopped (re-read its status). Not a
/// parameter change: comes without `NOTIFY_STATE`.
pub const NOTIFY_SIGGEN: u32 = 1 << 5;
/// An auxiliary output changed state or level (re-read the control
/// surfaces).
pub const NOTIFY_CS_AUX: u32 = 1 << 6;
/// IR learning finished: a code was captured or the window timed out.
pub const NOTIFY_IR_LEARN: u32 = 1 << 7;
/// Another host renamed an input source or the device (re-read the names:
/// `get_input_source_name`, `get_device_name`). New in the merged core.
pub const NOTIFY_NAMES: u32 = 1 << 8;
/// The device's notification stream ended (unplugged, MIDI port closed,
/// Link session over): the connection is dead and should be closed. Never
/// set after an intentional `disconnect`. New in the merged core.
pub const NOTIFY_DISCONNECTED: u32 = 1 << 9;
// -- WP C8 --
/// `DspiError::Refused { status }` for an extension-channel write the
/// device stalled (`REQ_EXTENSION`, WP C8): the extension channel has no
/// status byte, so this code stands for "refused".
pub const EXT_WRITE_REFUSED: u8 = 0xFF;
// -- WP C9 --
/// Spectrum analyser: band slots per channel (`RtaCapsInfo::centres`,
/// `RtaChannelFrame::avg_db` / `peak_db`), `RTA_MAX_BANDS` (rta_fft.h:42).
pub const RTA_MAX_BANDS: usize = dspi_proto::generated::rta::RTA_MAX_BANDS as usize;
/// Spectrum analyser: channel slots in `RtaSnapshot::frames` (the channel
/// mask is 16 bits).
pub const RTA_MAX_CHANNELS: usize = 16;
/// Spectrum analyser: bin slots in `RtaSnapshot::bins_db` (N/2 at the
/// largest transform, 1024 points).
pub const RTA_MAX_BINS: usize = 512;
/// Spectrum analyser tap: after the per-input PEQ, before the matrix.
pub const RTA_TAP_INPUT: u8 = dspi_proto::generated::rta::RTA_TAP_INPUT as u8;
/// Spectrum analyser tap: after output gain and delay.
pub const RTA_TAP_OUTPUT: u8 = dspi_proto::generated::rta::RTA_TAP_OUTPUT as u8;
/// dBFS of a silent band or bin (level byte 0 at the firmware's 0 dBFS
/// level 243, 0.5 dB steps): -121.5.
pub const RTA_FLOOR_DB: f32 = -121.5;
// -- WP C10 --

/// Signal generator config/status/caps layout version this core speaks
/// (`SIGGEN_CFG_VERSION`, siggen.h).
pub const SIGGEN_VERSION: u8 = 1;
/// Length of `SiggenCaps.types` (the old core's fixed table size).
pub const SIGGEN_TYPE_COUNT: usize = 15;
/// `SiggenConfig.flags`: bypass the outputs' crossover and PEQ.
pub const SIGGEN_FLAG_RAW: u8 = dspi_proto::packets::SiggenConfig::FLAG_RAW;
/// `SiggenConfig.flags`: noise gets an independent generator per channel.
pub const SIGGEN_FLAG_DECORR: u8 = dspi_proto::packets::SiggenConfig::FLAG_DECORR;
/// `SiggenConfig.flags`: play the masked outputs one at a time.
pub const SIGGEN_FLAG_WALK: u8 = dspi_proto::packets::SiggenConfig::FLAG_WALK;
/// `siggen_control` action: fade out, then idle.
pub const SIGGEN_CTL_STOP: u16 = dspi_proto::generated::siggen::SIGGEN_CTL_STOP;
/// `siggen_control` action: (re)start with the staged config.
pub const SIGGEN_CTL_START: u16 = dspi_proto::generated::siggen::SIGGEN_CTL_START;
/// `siggen_control` action: stop at once, no fade.
pub const SIGGEN_CTL_STOP_NOW: u16 = dspi_proto::generated::siggen::SIGGEN_CTL_STOP_NOW;

/// `fetch_stat`: PDM ring overruns since boot.
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
/// `fetch_stat`: temperature, centi-degrees C, signed (an `i16` widened to
/// `u32`).
pub const STAT_TEMPERATURE: u16 = 16;
/// `fetch_stat`: S/PDIF DMA starvations, all instances.
pub const STAT_STARVATION_TOTAL: u16 = 17;
/// `fetch_stat`: S/PDIF DMA starvations of instance 0; instances 1-3 follow
/// (18-21).
pub const STAT_STARVATION_FIRST: u16 = 18;
/// `fetch_stat`: USB audio ring overruns since boot.
pub const STAT_USB_RING_OVERRUNS: u16 = 22;
// -- WP C11 --
// -- WP C12 --
// -- WP C13 --
/// `config_import` option: also apply the master and user volume.
pub const IMPORT_VOLUMES: u32 = 1;
/// `config_import` option: also apply the hardware I/O and the limiters.
pub const IMPORT_HARDWARE: u32 = 2;
// -- WP C14 --
// -- WP C15 --
// -- WP F1 --
