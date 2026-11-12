//! Named numbers of the core's API.
//!
//! Sizes and ids the firmware defines are spelled as expressions over
//! `dspi_proto::generated` (built from the firmware headers) or other
//! `dspi_proto` items, so a header change moves them too; the rest are this
//! API's own values. The C header (`dspi-capi`'s `types/consts.rs`) and the
//! Kotlin and Swift constant files are generated from or checked against
//! this file: one `pub const` per line, and a value that is an expression
//! must also be listed in `dspi-capi`'s consts.

use dspi_proto::generated::{rta, siggen, status, tube, wire};
use dspi_proto::packets::SiggenConfig;

// ── compatibility ─────────────────────────────────────────────────────

/// `DspState.compat` before anything was read.
pub const COMPAT_UNKNOWN: u8 = 0;
/// `DspState.compat`: the device's wire format is one this build reads and writes.
pub const COMPAT_OK: u8 = 1;
/// `DspState.compat`: the firmware is older than this build supports.
pub const COMPAT_FIRMWARE_TOO_OLD: u8 = 2;
/// `DspState.compat`: the firmware is newer than this build knows.
pub const COMPAT_FIRMWARE_TOO_NEW: u8 = 3;

// ── sizes ─────────────────────────────────────────────────────────────

/// Channels in the per-channel vectors (inputs plus outputs, protocol maximum).
pub const MAX_CHANNELS: usize = wire::WIRE_MAX_CHANNELS as usize;
/// Input channels the protocol can describe.
pub const MAX_INPUTS: usize = wire::WIRE_MAX_INPUT_CHANNELS as usize;
/// Output channels the protocol can describe.
pub const MAX_OUTPUTS: usize = wire::WIRE_MAX_OUTPUT_CHANNELS as usize;
/// PEQ bands per channel in `DspState.filters` (the ones the firmware runs).
pub const BANDS_PER_CHANNEL: usize = 10;
/// PEQ band slots per channel in the bulk image (some are reserved).
pub const FIRMWARE_BANDS_PER_CHANNEL: usize = wire::WIRE_MAX_BANDS as usize;
/// Crossover slots per channel.
pub const MAX_XOVER_BANDS: usize = wire::WIRE_MAX_XOVER_BANDS as usize;
/// Preset slots on the device.
pub const MAX_PRESETS: usize = status::PRESET_SLOTS as usize;
/// Bytes of a channel name on the wire.
pub const CHANNEL_NAME_LEN: usize = wire::WIRE_NAME_LEN as usize;
/// Physical output pins the protocol can describe.
pub const MAX_PHYSICAL_OUTPUTS: usize = wire::WIRE_MAX_PIN_OUTPUTS as usize;
/// Bytes of a crossfeed pair map, one per stereo pair.
pub const CROSSFEED_MAP_PAIRS: usize = 4;

// ── defaults and markers ──────────────────────────────────────────────

/// The Q a new band starts with (Butterworth).
pub const DEFAULT_Q: f32 = 0.707;
/// `DspState.fw_beta` of the early 1.1.6 betas, which do not report their number.
pub const FW_BETA_EARLY: u8 = 0xFF;

// ── upmixer parameter ids ─────────────────────────────────────────────

/// Upmixer on/off.
pub const UPMIX_PARAM_ENABLED: u8 = 0;
/// Centre extraction mode.
pub const UPMIX_PARAM_CENTER_MODE: u8 = 1;
/// Surround extraction mode.
pub const UPMIX_PARAM_SURROUND_MODE: u8 = 2;
/// Steering strength.
pub const UPMIX_PARAM_STRENGTH: u8 = 3;
/// Centre width.
pub const UPMIX_PARAM_WIDTH: u8 = 4;
/// Steering threshold.
pub const UPMIX_PARAM_THRESHOLD: u8 = 5;
/// Steering attack.
pub const UPMIX_PARAM_ATTACK: u8 = 6;
/// Steering release.
pub const UPMIX_PARAM_RELEASE: u8 = 7;
/// High pass in front of the direction detector.
pub const UPMIX_PARAM_DETECTOR_HPF: u8 = 8;
/// Delay of the surround channels.
pub const UPMIX_PARAM_SURROUND_DELAY: u8 = 9;
/// High pass on the surround channels.
pub const UPMIX_PARAM_SURROUND_HPF: u8 = 10;
/// Low pass on the surround channels.
pub const UPMIX_PARAM_SURROUND_LPF: u8 = 11;
/// Surround decorrelation.
pub const UPMIX_PARAM_DECORRELATION: u8 = 12;
/// Centre presence bell.
pub const UPMIX_PARAM_PRESENCE: u8 = 13;
/// Length of `DspState.upmix`.
pub const UPMIX_PARAM_COUNT: usize = 14;

// ── subharmonic synth parameter ids ───────────────────────────────────

/// Synth on/off.
pub const SUBHARM_PARAM_ENABLED: u8 = 0;
/// Outputs it feeds.
pub const SUBHARM_PARAM_MASK: u8 = 1;
/// Lower edge of the detection band.
pub const SUBHARM_PARAM_LOW: u8 = 2;
/// Upper edge of the detection band.
pub const SUBHARM_PARAM_HIGH: u8 = 3;
/// Top of the generated band.
pub const SUBHARM_PARAM_TOP: u8 = 4;
/// Level of the generated signal.
pub const SUBHARM_PARAM_BOOST: u8 = 5;
/// Which octave(s) are generated.
pub const SUBHARM_PARAM_SELECT: u8 = 6;
/// Effect depth.
pub const SUBHARM_PARAM_DEPTH: u8 = 7;
/// Envelope hold.
pub const SUBHARM_PARAM_HOLD: u8 = 8;
/// Output ceiling.
pub const SUBHARM_PARAM_CEILING: u8 = 9;
/// Stereo link.
pub const SUBHARM_PARAM_LINK: u8 = 10;
/// Length of `DspState.subharm`.
pub const SUBHARM_PARAM_COUNT: usize = 11;

// ── tube modeller and psychoacoustic bass ─────────────────────────────

/// Length of `DspState.tube`.
pub const TUBE_PARAM_COUNT: usize = tube::TUBE_NUM_PARAMS as usize;
/// Psychoacoustic bass cutoff.
pub const PSYBASS_PARAM_CUTOFF: u8 = 0;
/// Harmonics level.
pub const PSYBASS_PARAM_HARMONICS: u8 = 1;
/// Drive.
pub const PSYBASS_PARAM_DRIVE: u8 = 2;
/// Even/odd character.
pub const PSYBASS_PARAM_CHARACTER: u8 = 3;
/// Level of the original low band.
pub const PSYBASS_PARAM_ORIGINAL: u8 = 4;
/// Number of psychoacoustic bass parameters.
pub const PSYBASS_PARAM_COUNT: usize = 5;

// ── flash results ─────────────────────────────────────────────────────

/// Saving to flash worked.
pub const FLASH_OK: u8 = 0;
/// The flash write failed.
pub const FLASH_ERR_WRITE: u8 = 1;
/// Nothing stored to load.
pub const FLASH_ERR_NO_DATA: u8 = 2;
/// Stored data failed its checksum.
pub const FLASH_ERR_CRC: u8 = 3;

// ── notification flags ────────────────────────────────────────────────

/// `NotifyResult.flags`: a setting changed; the state was updated.
pub const NOTIFY_STATE: u32 = 1 << 0;
/// The response curves changed (filters, gains, bypass, output enable).
pub const NOTIFY_CURVES: u32 = 1 << 1;
/// The whole state was read again (preset load, bulk write, lost events).
pub const NOTIFY_REFRESHED: u32 = 1 << 2;
/// The host's input channel count changed.
pub const NOTIFY_INPUT_FORMAT: u32 = 1 << 3;
/// A preset was loaded on the device.
pub const NOTIFY_PRESET: u32 = 1 << 4;
/// The signal generator started or stopped; comes without `NOTIFY_STATE`.
pub const NOTIFY_SIGGEN: u32 = 1 << 5;
/// A control-surface auxiliary output changed.
pub const NOTIFY_CS_AUX: u32 = 1 << 6;
/// The IR learner has something new.
pub const NOTIFY_IR_LEARN: u32 = 1 << 7;
/// The device name or an input source name changed.
pub const NOTIFY_NAMES: u32 = 1 << 8;
/// The device went away.
pub const NOTIFY_DISCONNECTED: u32 = 1 << 9;

// ── extension channel ─────────────────────────────────────────────────

/// Status of an extension write the device stalled.
pub const EXT_WRITE_REFUSED: u8 = 0xFF;

// ── analyser ──────────────────────────────────────────────────────────

/// Most bands in an analyser frame.
pub const RTA_MAX_BANDS: usize = rta::RTA_MAX_BANDS as usize;
/// Most channels in an analyser snapshot.
pub const RTA_MAX_CHANNELS: usize = 16;
/// Most raw FFT bins in a snapshot.
pub const RTA_MAX_BINS: usize = 512;
/// Measure after the input PEQ, before the matrix.
pub const RTA_TAP_INPUT: u8 = rta::RTA_TAP_INPUT as u8;
/// Measure at the outputs, after gain and delay.
pub const RTA_TAP_OUTPUT: u8 = rta::RTA_TAP_OUTPUT as u8;
/// The level a silent band reads, in dBFS.
pub const RTA_FLOOR_DB: f32 = -121.5;

// ── signal generator ──────────────────────────────────────────────────

/// Version of the signal generator's configuration block.
pub const SIGGEN_VERSION: u8 = siggen::SIGGEN_CFG_VERSION as u8;
/// Signal types the generator can describe.
pub const SIGGEN_TYPE_COUNT: usize = 15;
/// `SiggenConfig.flags`: skip the channel's crossover and PEQ.
pub const SIGGEN_FLAG_RAW: u8 = SiggenConfig::FLAG_RAW;
/// `SiggenConfig.flags`: an independent noise generator per channel.
pub const SIGGEN_FLAG_DECORR: u8 = SiggenConfig::FLAG_DECORR;
/// `SiggenConfig.flags`: play the channels one after another.
pub const SIGGEN_FLAG_WALK: u8 = SiggenConfig::FLAG_WALK;
/// Control code: stop after the current burst.
pub const SIGGEN_CTL_STOP: u16 = siggen::SIGGEN_CTL_STOP;
/// Control code: start.
pub const SIGGEN_CTL_START: u16 = siggen::SIGGEN_CTL_START;
/// Control code: stop at once.
pub const SIGGEN_CTL_STOP_NOW: u16 = siggen::SIGGEN_CTL_STOP_NOW;

// ── statistics counter ids ────────────────────────────────────────────

/// PDM ring buffer overruns.
pub const STAT_PDM_RING_OVERRUNS: u16 = 3;
/// PDM ring buffer underruns.
pub const STAT_PDM_RING_UNDERRUNS: u16 = 4;
/// PDM DMA overruns.
pub const STAT_PDM_DMA_OVERRUNS: u16 = 5;
/// PDM DMA underruns.
pub const STAT_PDM_DMA_UNDERRUNS: u16 = 6;
/// S/PDIF output overruns.
pub const STAT_SPDIF_OVERRUNS: u16 = 7;
/// S/PDIF output underruns.
pub const STAT_SPDIF_UNDERRUNS: u16 = 8;
/// System clock in Hz.
pub const STAT_CLOCK_HZ: u16 = 13;
/// Core voltage in mV.
pub const STAT_CORE_MV: u16 = 14;
/// Sample rate in Hz.
pub const STAT_SAMPLE_RATE: u16 = 15;
/// Chip temperature.
pub const STAT_TEMPERATURE: u16 = 16;
/// Audio starvation events, total.
pub const STAT_STARVATION_TOTAL: u16 = 17;
/// Audio starvation events, first stage.
pub const STAT_STARVATION_FIRST: u16 = 18;
/// USB receive ring overruns.
pub const STAT_USB_RING_OVERRUNS: u16 = 22;

// ── preset file import options ────────────────────────────────────────

/// Import also applies the master and user volume.
pub const IMPORT_VOLUMES: u32 = 1;
/// Import also applies the hardware settings (pins, interfaces).
pub const IMPORT_HARDWARE: u32 = 2;

// ── more notification flags ───────────────────────────────────────────

/// The state could not be re-read after retries: it may be out of date.
pub const NOTIFY_STALE: u32 = 1 << 10;
/// An input or output stream changed state (ADAT, I2S slave, ...).
pub const NOTIFY_STREAMS: u32 = 1 << 11;
// -- WP P0W --
/// The change history moved: a change, an undo or a redo was journalled, or
/// a new connection started a new history (`DspiCore::history`,
/// `can_undo`, `can_redo` may answer differently).
pub const NOTIFY_HISTORY: u32 = 1 << 12;
