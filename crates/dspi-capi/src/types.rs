//! The `#[repr(C)]` records of the C ABI and their conversions (WP Q1).
//!
//! Every struct here has the old desktop core's layout, field for field
//! (`crates/dspi-core-linux`, header `dspi_core.h`): same names, same order,
//! same types, so the Qt app compiles and runs unchanged. The tests
//! (`types/tests_wp_q1.rs`) check `size_of`, `align_of` and every
//! `offset_of` against the old header with the C compiler, and diff the
//! generated `include/dspi_core.h` against the old one.
//!
//! The records of `dspi_core::records` carry `Vec`s and `String`s where these
//! carry fixed arrays and NUL-padded buffers. The conversions never panic: a
//! short vector leaves the rest of the array at zero (or the documented
//! floor), a long one is cut to the array, and a string is cut at a
//! character boundary so that its buffer always ends in at least one NUL.
//!
//! The one record that is not copied out per call is [`DspState`] (about
//! 12 KB): `dspi_get_state` returns a pointer to one cached per core handle
//! and refreshed on each call (WP Q2a).
//!
//! Channel indices are firmware *wire* indices:
//! `[ inputs 0..num_input_channels-1 ][ outputs num_input_channels.. ]`.
//! Output-relative fields (matrix columns, output gain/mute/delay, limiter)
//! take an output index 0..num_output_channels-1.

use std::ffi::{c_char, c_void};

use dspi_core::records as r;

mod consts;
pub use consts::*;

// ── Callbacks ─────────────────────────────────────────────────────────

/// Hot-plug event callback: `event` 0 = arrived, 1 = departed; `serial` is
/// the NUL-terminated ASCII device serial, valid during the call only.
pub type DeviceEventCallback =
    extern "C" fn(event: u8, serial: *const c_char, user_data: *mut c_void);

/// Called from the notification thread when packets are waiting. It must
/// only schedule `dspi_process_notifications` on the GUI thread.
pub type NotifyCallback = extern "C" fn(user_data: *mut c_void);

// ── types.rs ──────────────────────────────────────────────────────────

/// Parameters for a single filter band (PEQ or crossover).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FilterParams {
    /// Firmware filter type (`FILTER_*`).
    pub filter_type: u8,
    /// User bypass: the band keeps its settings but is not processed.
    pub bypass: bool,
    /// Centre or corner frequency in Hz.
    pub freq: f32,
    /// Quality factor (ignored by first-order types).
    pub q: f32,
    /// Gain in dB; for the Linkwitz Transform, the target frequency fp in Hz.
    pub gain: f32,
    /// Linkwitz Transform target Q (ignored by every other type).
    pub qp: f32,
}

/// External-clock lock of the I2S input in slave mode, or of the ADAT input.
/// state: I2S 0 inactive, 1 acquiring, 2 relocking, 3 locked;
///        ADAT 0 inactive, 1 acquiring, 2 syncing, 3 locked, 4 relocking.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputLockStatus {
    /// Lock state (see the struct docs).
    pub state: u8,
    /// 0 = master (DSPi drives the clock), 1 = slave.
    pub clock_mode: u8,
    /// Rate locked to (Hz, 0 until locked).
    pub detected_rate: u32,
    /// Raw measured rate in Hz.
    pub measured_hz: u32,
    /// ADAT: the rate is supported (44.1/48 kHz).
    pub rate_ok: bool,
    /// Locks since the input started (saturates at 255).
    pub lock_count: u8,
    /// Lock losses since the input started (saturates at 255).
    pub loss_count: u8,
}

/// ADAT bulk output state (REQ_GET_ADAT_STATUS, RP2350).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AdatOutStatus {
    /// The output is configured on.
    pub enabled: bool,
    /// Streaming now (suspended above 48 kHz).
    pub active: bool,
    /// Output GPIO.
    pub pin: u8,
    /// The current rate is one ADAT carries.
    pub rate_ok: bool,
    /// Resynchronisations since start.
    pub resync_count: u16,
    /// Sample slips since start.
    pub slip_count: u16,
}

/// LG Sound Sync live state (REQ_GET_LG_SOUND_SYNC_STATUS).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LgStatus {
    /// The decoder is enabled.
    pub enabled: bool,
    /// An LG TV's volume signalling is being decoded.
    pub present: bool,
    /// TV volume 0..100; 0xFF until one has been decoded.
    pub volume: u8,
    /// The TV is muted.
    pub muted: bool,
}

/// UART control interface config (REQ_GET/SET_UART_CONFIG). Device-level,
/// stored in the flash directory, not in the bulk image.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UartConfig {
    /// The interface is on.
    pub enabled: bool,
    /// TX GPIO.
    pub tx_pin: u8,
    /// RX GPIO.
    pub rx_pin: u8,
    /// Push change notifications to the controller.
    pub notify: bool,
    /// Baud rate.
    pub baud: u32,
}

/// I2C target control interface config (REQ_GET/SET_I2C_CONFIG).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct I2cConfig {
    /// The interface is on.
    pub enabled: bool,
    /// SDA GPIO.
    pub sda_pin: u8,
    /// SCL GPIO.
    pub scl_pin: u8,
    /// 7-bit target address, 0x08..0x77.
    pub address: u8,
}

/// Control interface outcome and run state (REQ_GET_CTRL_IFACE_STATUS).
/// `*_last_status`: 0 OK, 1 invalid pin, 2 pin in use, 5 invalid parameter.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CtrlIfaceStatus {
    /// Outcome of the last UART configuration.
    pub uart_last_status: u8,
    /// The UART interface is running.
    pub uart_live: bool,
    /// Outcome of the last I2C configuration.
    pub i2c_last_status: u8,
    /// The I2C interface is running.
    pub i2c_live: bool,
    /// Control protocol version the firmware speaks.
    pub protocol_version: u8,
}

/// Live upmixer state (REQ_UPMIX_GET_STATUS).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UpmixStatus {
    /// True while the upmixer is processing audio.
    pub active: bool,
    /// Why it is idle: 0 active, 1 disabled, 2 input not stereo, 3 rate above 48 kHz.
    pub parked_reason: u8,
    /// Running L/R correlation, -1..1 (0 with a passive centre).
    pub correlation: f32,
    /// Running L/R balance, -1..1.
    pub balance: f32,
    /// Current gains of the derived channels, 0..1.
    pub center_gain: f32,
    /// Left surround gain, 0..1.
    pub ls_gain: f32,
    /// Right surround gain, 0..1.
    pub rs_gain: f32,
}

/// System status from the device (REQ_GET_STATUS wValue = 9).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SystemStatus {
    /// Peak level per wire channel, 0.0..1.0.
    pub peaks: [f32; MAX_CHANNELS],
    /// Core 0 load in %.
    pub cpu0: u8,
    /// Core 1 load in %.
    pub cpu1: u8,
    /// Sticky clip flags, one bit per wire channel.
    pub clip_flags: u32,
    /// Live active input channel count (source-aware).
    pub active_input_channels: u8,
    /// Wire channels (inputs + outputs).
    pub num_channels: u8,
}

/// Information about a discovered DSPi device.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceInfo {
    /// NUL-terminated ASCII serial number.
    pub serial: [u8; 64],
    /// Bytes of `serial` before the NUL.
    pub serial_len: u32,
    /// Bus location id (0 where the platform has none).
    pub location_id: u32,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            serial: [0; 64],
            serial_len: 0,
            location_id: 0,
        }
    }
}

/// Platform and firmware identification (REQ_GET_PLATFORM, 7-byte reply).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformInfo {
    /// 0 = RP2040, 1 = RP2350.
    pub platform_id: u8,
    /// Output channels of this build.
    pub num_output_channels: u8,
    /// Firmware major version.
    pub fw_major: u8,
    /// Firmware minor version.
    pub fw_minor: u8,
    /// Firmware patch version.
    pub fw_patch: u8,
    /// Pre-release ordinal: 0 = final release, N = beta N.
    pub fw_beta: u8,
}

/// Preset directory (REQ_PRESET_GET_DIR, 7-byte reply).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PresetDirectory {
    /// Bitmask of occupied preset slots (10 slots).
    pub occupied_mask: u16,
    /// 0 = load the specified default slot, 1 = load the last active slot.
    pub startup_mode: u8,
    /// Slot loaded at startup in mode 0.
    pub default_slot: u8,
    /// Last slot loaded or saved.
    pub last_active: u8,
    /// 0 = output config is stored independently, 1 = it travels with presets.
    pub output_config_mode: u8,
    /// 0 = master volume is stored independently, 1 = it travels with presets.
    pub master_volume_mode: u8,
}

// ── state.rs ──────────────────────────────────────────────────────────

/// Complete DSP parameter state for the connected device.
/// Owned by the core handle; the GUI reads it through `dspi_get_state`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DspState {
    // ── Device / firmware ───────────────────────────────────────────
    /// 0 = RP2040, 1 = RP2350.
    pub platform_id: u8,
    /// Wire channels (inputs + outputs).
    pub num_channels: u8,
    /// Input channels.
    pub num_input_channels: u8,
    /// Output channels.
    pub num_output_channels: u8,
    /// Wire format version of the bulk image.
    pub format_version: u8,
    /// Firmware major version.
    pub fw_major: u8,
    /// Firmware minor version.
    pub fw_minor: u8,
    /// Firmware patch version.
    pub fw_patch: u8,
    /// Firmware beta number, 0 = final.
    pub fw_beta: u8,
    /// `COMPAT_*`.
    pub compat: u8,

    // ── Global ──────────────────────────────────────────────────────
    /// Legacy global preamp (mirrors input 0's preamp on current firmware).
    pub preamp_db: f32,
    /// Global DSP bypass.
    pub bypass: bool,

    // ── Per-input preamp ────────────────────────────────────────────
    /// Preamp per input, dB.
    pub input_preamp_db: [f32; MAX_INPUTS],

    // ── Volume ──────────────────────────────────────────────────────
    /// Device master volume: −128 = mute sentinel, −127..0 dB.
    pub master_volume_db: f32,
    /// 0 = independent of presets, 1 = saved with presets.
    pub master_volume_mode: u8,
    /// User volume, dB.
    pub user_volume_db: f32,
    /// User mute.
    pub user_mute: bool,

    // ── Loudness ────────────────────────────────────────────────────
    /// Loudness compensation on.
    pub loudness_enabled: bool,
    /// Reference level, dB SPL.
    pub loudness_ref_spl: f32,
    /// Intensity, %.
    pub loudness_intensity: f32,
    /// Bit k: loudness processes output k.
    pub loudness_output_mask: u16,

    // ── Crossfeed ───────────────────────────────────────────────────
    /// Crossfeed on.
    pub crossfeed_enabled: bool,
    /// Crossfeed preset id.
    pub crossfeed_preset: u8,
    /// Custom cutoff, Hz.
    pub crossfeed_freq: f32,
    /// Custom feed level, dB.
    pub crossfeed_feed: f32,
    /// Interaural time delay on.
    pub crossfeed_itd: bool,
    /// Bit p: crossfeed runs on output pair p.
    pub crossfeed_output_pair_mask: u8,

    // ── Per-channel delays (wire channel index) ─────────────────────
    /// Delay per wire channel, ms.
    pub channel_delays: [f32; MAX_CHANNELS],

    // ── Matrix mixer [input][output] ────────────────────────────────
    /// Crosspoint on.
    pub matrix_routing: [[bool; MAX_OUTPUTS]; MAX_INPUTS],
    /// Crosspoint gain, dB.
    pub matrix_gain: [[f32; MAX_OUTPUTS]; MAX_INPUTS],
    /// Crosspoint polarity inverted.
    pub matrix_invert: [[bool; MAX_OUTPUTS]; MAX_INPUTS],

    // ── Outputs ─────────────────────────────────────────────────────
    /// Output enabled.
    pub output_enabled: [bool; MAX_OUTPUTS],
    /// Output muted.
    pub output_muted: [bool; MAX_OUTPUTS],
    /// Output gain, dB.
    pub output_gain_db: [f32; MAX_OUTPUTS],
    /// Output delay, ms.
    pub output_delay_ms: [f32; MAX_OUTPUTS],

    // ── Output limiter (per output) ─────────────────────────────────
    /// Limiter on.
    pub limiter_enabled: [bool; MAX_OUTPUTS],
    /// 0 = unlinked, 1..4 = link group.
    pub limiter_link_group: [u8; MAX_OUTPUTS],
    /// Threshold, dBFS.
    pub limiter_threshold_db: [f32; MAX_OUTPUTS],
    /// Release time, ms.
    pub limiter_release_ms: [f32; MAX_OUTPUTS],

    // ── Pin configuration ───────────────────────────────────────────
    /// Physical output slots in use.
    pub num_pin_outputs: u8,
    /// GPIO per physical output slot.
    pub output_pins: [u8; MAX_PHYSICAL_OUTPUTS],
    /// 0 = output config stored independently, 1 = saved with presets.
    pub output_config_mode: u8,

    // ── EQ (wire channel index) ─────────────────────────────────────
    /// PEQ bands [wire channel][band].
    pub filters: [[FilterParams; BANDS_PER_CHANNEL]; MAX_CHANNELS],
    /// Crossover bands; input rows are unused (crossovers are output-only).
    pub xover: [[FilterParams; MAX_XOVER_BANDS]; MAX_CHANNELS],

    // ── Channel names (wire channel index) ──────────────────────────
    /// NUL-padded UTF-8 name per wire channel.
    pub channel_names: [[u8; CHANNEL_NAME_LEN]; MAX_CHANNELS],

    // ── Volume leveller ─────────────────────────────────────────────
    /// Leveller on.
    pub leveller_enabled: bool,
    /// 0 = slow, 1 = medium, 2 = fast.
    pub leveller_speed: u8,
    /// Lookahead on.
    pub leveller_lookahead: bool,
    /// Amount, %.
    pub leveller_amount: f32,
    /// Maximum gain, dB.
    pub leveller_max_gain_db: f32,
    /// Gate threshold, dBFS.
    pub leveller_gate_db: f32,
    /// Bit k: input k feeds the detector.
    pub leveller_detector_mask: u8,
    /// Bit k: gain applied to input k.
    pub leveller_apply_mask: u8,

    // ── Input / misc ────────────────────────────────────────────────
    /// 0 = USB, 1 = S/PDIF, 2 = I2S, ...
    pub input_source: u8,
    /// LG Sound Sync decoding on.
    pub lg_sound_sync_enabled: bool,
    /// Bit k: S/PDIF input k+1 is enabled (input 1 is always on).
    pub spdif_inputs_enabled: u8,
    /// ADAT input on.
    pub adat_input_enabled: bool,
    /// RX pin of each S/PDIF input (1-4).
    pub spdif_rx_pins: [u8; 4],
    /// Data pin of each I2S input pair; active pairs = channels / 2.
    pub i2s_rx_pins: [u8; 4],
    /// I2S input channels.
    pub i2s_input_channels: u8,
    /// Master-mode rate shared by I2S and ADAT: 0 = 44.1k, 1 = 48k, 2 = 96k.
    pub i2s_input_rate: u8,
    /// 0 = master (DSPi drives BCK/LRCLK), 1 = slave.
    pub i2s_clock_mode: u8,
    /// ADAT input data pin, 0xFF = not set (it ships unset).
    pub adat_input_pin: u8,
    /// ADAT input clock: 0 = master, 1 = slave.
    pub adat_input_clock_mode: u8,

    // ── Psychoacoustic bass ─────────────────────────────────────────
    /// Psychoacoustic bass on.
    pub psybass_enabled: bool,
    /// Bit k: psybass processes output k.
    pub psybass_output_mask: u16,
    /// Cutoff, Hz.
    pub psybass_cutoff_hz: f32,
    /// Harmonics level, dB.
    pub psybass_harmonics_db: f32,
    /// Drive, dB.
    pub psybass_drive_db: f32,
    /// Character, %.
    pub psybass_character_pct: f32,
    /// Original signal level, dB.
    pub psybass_original_db: f32,

    // ── Stereo upmixer (RP2350 only) ────────────────────────────────
    /// By `UPMIX_PARAM_*` id; enable and modes as whole numbers.
    pub upmix: [f32; UPMIX_PARAM_COUNT],

    // ── Subharmonic synthesizer ─────────────────────────────────────
    /// By `SUBHARM_PARAM_*` id; flags, mask and mode as whole numbers.
    pub subharm: [f32; SUBHARM_PARAM_COUNT],
    /// Program muted on the masked outputs. Runtime only: not in the bulk
    /// image, never saved.
    pub subharm_solo: bool,

    // ── Tube modeller ───────────────────────────────────────────────
    /// By `TUBE_PARAM_*` index; flags, mask and enums as whole numbers.
    pub tube: [f32; TUBE_PARAM_COUNT],

    // ── Output types and clocks (i2s_config / adat_config sections) ─
    /// Per S/PDIF-or-I2S slot: 0 = S/PDIF, 1 = I2S.
    pub output_types: [u8; 4],
    /// I2S bit clock (LRCLK is the next GPIO).
    pub i2s_bck_pin: u8,
    /// I2S bit clock of the slave pair in split mode.
    pub i2s_bck_pin_slave: u8,
    /// 0 = master and slave share the clock pins, 1 = separate pins.
    pub i2s_clock_pin_mode: u8,
    /// MCK output on.
    pub mck_enabled: bool,
    /// MCK GPIO.
    pub mck_pin: u8,
    /// 0 = 128 x fs, 1 = 256 x fs.
    pub mck_multiplier: u8,
    /// ADAT optical output (RP2350).
    pub adat_out_enabled: bool,
    /// ADAT output GPIO.
    pub adat_out_pin: u8,

    // ── DAC hardware mute (device-level; in the bulk image) ────────
    /// The firmware has the DAC mute feature.
    pub dac_mute_supported: bool,
    /// DAC mute on.
    pub dac_mute_enabled: bool,
    /// The mute line is active low.
    pub dac_mute_active_low: bool,
    /// GPIO, 0xFF = none.
    pub dac_mute_pin: u8,
    /// Hold time, ms.
    pub dac_mute_hold_ms: u16,
    /// Release time, ms.
    pub dac_mute_release_ms: u16,

    // ── UART / I2C control interfaces (flash directory, not bulk) ──
    /// The firmware has the control interfaces.
    pub ctrl_iface_supported: bool,
    /// UART interface config.
    pub uart: UartConfig,
    /// I2C interface config.
    pub i2c: I2cConfig,
    /// Control interface status.
    pub ctrl_status: CtrlIfaceStatus,

    // ── Core 1 mode ─────────────────────────────────────────────────
    /// What core 1 runs (firmware `Core1Mode`).
    pub core1_mode: u8,

    // ── Presets ─────────────────────────────────────────────────────
    /// Bitmask of occupied preset slots.
    pub preset_occupied: u16,
    /// NUL-padded UTF-8 name per preset slot.
    pub preset_names: [[u8; CHANNEL_NAME_LEN]; MAX_PRESETS],
    /// Active preset slot.
    pub active_preset_slot: u8,
    /// 0 = load the default slot at startup, 1 = the last active one.
    pub preset_startup_mode: u8,
    /// Slot loaded at startup in mode 0.
    pub preset_default_slot: u8,

    // ── Raw bulk image ──────────────────────────────────────────────
    /// True once a bulk image has been read from the device.
    pub bulk_valid: bool,
    /// The last bulk image read, kept so unmodeled sections round-trip.
    pub bulk_raw: [u8; BULK_PARAMS_SIZE],
}

// ── notify.rs ─────────────────────────────────────────────────────────

/// What a batch of notifications changed.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NotifyResult {
    /// `NOTIFY_*` bits.
    pub flags: u32,
    /// Wire channels whose PEQ or crossover bands changed (bit per channel).
    pub filter_channels: u32,
}

// ── rta.rs ────────────────────────────────────────────────────────────

/// What the GUI wants. A zero mask releases the analyser.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RtaRequest {
    /// RTA_TAP_INPUT or RTA_TAP_OUTPUT.
    pub tap: u8,
    /// Bit i = channel i at that tap (input index or output index).
    pub mask: u16,
    /// Also read the FFT bins (single-channel selections only).
    pub wants_bins: bool,
    /// FFT size as a power of two.
    pub fft_order: u8,
    /// Averaging time, ms.
    pub avg_ms: u16,
    /// Peak hold decay, dB/s.
    pub peak_decay_db_s: u8,
}

/// The device's analyser. `supported` is false until probed, and stays
/// false on firmware without one.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaCapsInfo {
    /// The firmware has an analyser.
    pub supported: bool,
    /// Input channels it can tap.
    pub input_channels: u8,
    /// Output channels it can tap.
    pub output_channels: u8,
    /// Smallest FFT order.
    pub order_min: u8,
    /// Largest FFT order.
    pub order_max: u8,
    /// Default FFT order.
    pub order_default: u8,
    /// Bands computed by the bass filter bank.
    pub bass_bands: u8,
    /// Bands in total.
    pub max_bands: u8,
    /// Dynamic range of the FFT bands, dB.
    pub dynamic_range_db: u8,
    /// Dynamic range of the bass bands, dB.
    pub bass_dynamic_range_db: u16,
    /// Nominal band centres in Hz (max_bands of them).
    pub centres: [u16; RTA_MAX_BANDS],
}

impl Default for RtaCapsInfo {
    fn default() -> Self {
        Self {
            supported: false,
            input_channels: 0,
            output_channels: 0,
            order_min: 0,
            order_max: 0,
            order_default: 0,
            bass_bands: 0,
            max_bands: 0,
            dynamic_range_db: 0,
            bass_dynamic_range_db: 0,
            centres: [0; RTA_MAX_BANDS],
        }
    }
}

/// One channel's band levels in dBFS. A silent channel (no audio, or not
/// published yet) reads the floor.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaChannelFrame {
    /// Channel at the snapshot's tap.
    pub channel: u8,
    /// Bands carried.
    pub n_bands: u8,
    /// No level yet: the bands read the floor.
    pub silent: bool,
    /// Averaged level per band, dBFS.
    pub avg_db: [f32; RTA_MAX_BANDS],
    /// Peak-hold level per band, dBFS.
    pub peak_db: [f32; RTA_MAX_BANDS],
}

impl Default for RtaChannelFrame {
    /// A silent frame at `RTA_FLOOR_DB`, as the old core's.
    fn default() -> Self {
        Self {
            channel: 0,
            n_bands: 0,
            silent: true,
            avg_db: [RTA_FLOOR_DB; RTA_MAX_BANDS],
            peak_db: [RTA_FLOOR_DB; RTA_MAX_BANDS],
        }
    }
}

/// The latest picture and telemetry. `version` changes only when what is
/// drawn changed; `telemetry_version` when the status line did.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaSnapshot {
    /// Changes when what is drawn changed.
    pub version: u32,
    /// Changes when the telemetry changed.
    pub telemetry_version: u32,
    /// RTA_TAP_INPUT or RTA_TAP_OUTPUT.
    pub tap: u8,
    /// Valid entries of `frames`.
    pub count: u8,
    /// Analysed channels, packed in ascending channel order.
    pub frames: [RtaChannelFrame; RTA_MAX_CHANNELS],
    /// Bands with a level at this rate and transform size (bit per band).
    pub populated: u64,
    /// Averaged and 1/6-octave smoothed bins in dBFS; bin k at
    /// k * bins_sample_rate / (2 * bins_count). 0 = none.
    pub bins_count: u16,
    /// Channel the bins belong to.
    pub bins_channel: u8,
    /// Sample rate of the bins, Hz.
    pub bins_sample_rate: u32,
    /// The bins, dBFS (first `bins_count` valid).
    pub bins_db: [f32; RTA_MAX_BINS],
    // Telemetry
    /// The analyser is running.
    pub running: bool,
    /// Channels being analysed.
    pub live_count: u8,
    /// Mask of the channels being analysed.
    pub live_mask: u16,
    /// Frames per second.
    pub frames_per_s: u16,
    /// FFT CPU time, µs per second.
    pub busy_us_per_s: u16,
    /// CPU time of the last frame, µs.
    pub last_frame_us: u16,
    /// Bass bank CPU time, µs per second.
    pub bass_busy_us_per_s: u16,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Lowest band carrying a level, 0xFF = none.
    pub first_band: u8,
    /// FFT order in use.
    pub fft_order: u8,
    /// The device would not take the requested configuration.
    pub config_rejected: bool,
    /// How often each channel gets a new frame, in seconds.
    pub refresh_interval_s: f32,
}

impl Default for RtaSnapshot {
    /// Silent frames at the floor and no bins, as the old core's.
    fn default() -> Self {
        Self {
            version: 0,
            telemetry_version: 0,
            tap: 0,
            count: 0,
            frames: [RtaChannelFrame::default(); RTA_MAX_CHANNELS],
            populated: 0,
            bins_count: 0,
            bins_channel: 0,
            bins_sample_rate: 0,
            bins_db: [RTA_FLOOR_DB; RTA_MAX_BINS],
            running: false,
            live_count: 0,
            live_mask: 0,
            frames_per_s: 0,
            busy_us_per_s: 0,
            last_frame_us: 0,
            bass_busy_us_per_s: 0,
            sample_rate: 0,
            first_band: 0xFF,
            fft_order: 0,
            config_rejected: false,
            refresh_interval_s: 0.0,
        }
    }
}

// ── siggen.rs ─────────────────────────────────────────────────────────

/// One signal's settings. `p` holds the type's parameters (see the caps).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenConfig {
    /// Signal type id.
    pub signal_type: u8,
    /// Bit i = output i.
    pub channel_mask: u16,
    /// Polarity-inverted subset of channel_mask.
    pub invert_mask: u16,
    /// SIGGEN_FLAG_*.
    pub flags: u8,
    /// Peak level, dBFS (-120..0).
    pub level_db: f32,
    /// Continuous: play time (0 = forever), or per-output dwell when walking;
    /// sweep: one sweep. Unused by patterns.
    pub duration_ms: u32,
    /// Sweeps, pattern periods or walk passes (0 = forever).
    pub repeat: u16,
    /// Gap between repeats, ms.
    pub gap_ms: u16,
    /// Type-specific parameters.
    pub p: [f32; 4],
}

/// What the generator is doing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenStatus {
    /// 0 idle, 1 fading in, 2 running, 3 gap, 4 fading out.
    pub state: u8,
    /// Signal type id.
    pub signal_type: u8,
    /// Output being played when walking, else 0xFF.
    pub active_channel: u8,
    /// Time since start, ms.
    pub elapsed_ms: u32,
    /// Completed repeats.
    pub cycles_done: u16,
    /// Of the last stop: 0 none, 1 host, 2 completed, 3 preset load, 4 reconfigured.
    pub stop_reason: u8,
    /// Sweep frequency now, else 0.
    pub current_freq: f32,
}

/// One parameter of a signal type.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenParam {
    /// 0 unused, 1 Hz, 2 ms, 3 cycles, 4 count, 5 ratio, 6 pattern.
    pub semantic: u8,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// Default value.
    pub def: f32,
}

/// One signal type the generator offers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenTypeInfo {
    /// Signal type id.
    pub id: u8,
    /// Short firmware name, NUL-terminated.
    pub name: [u8; 9],
    /// 0 continuous, 1 sweep, 2 pattern.
    pub timing: u8,
    /// The type's parameters (`p` of [`SiggenConfig`]).
    pub params: [SiggenParam; 4],
}

/// The generator on this device. `supported` false without one.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenCaps {
    /// The firmware has a generator.
    pub supported: bool,
    /// Valid entries of `types`.
    pub type_count: u8,
    /// Outputs it can drive.
    pub output_channels: u8,
    /// Most tones of a multitone signal.
    pub multitone_max: u8,
    /// Bit t: type id t is valid.
    pub valid_mask: u16,
    /// The signal types.
    pub types: [SiggenTypeInfo; SIGGEN_TYPE_COUNT],
}

// ── stats.rs ──────────────────────────────────────────────────────────

/// One S/PDIF output's buffer pipeline.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpdifBufferStats {
    /// Free buffers.
    pub free: u8,
    /// Buffers prepared.
    pub prepared: u8,
    /// Buffers playing.
    pub playing: u8,
    /// Consumer pipeline fill now, in %.
    pub fill_pct: u8,
    /// Lowest fill since the watermarks were reset, %.
    pub min_pct: u8,
    /// Highest fill since the watermarks were reset, %.
    pub max_pct: u8,
}

/// The PDM output's DMA and ring buffers, fill now / lowest / highest in %.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PdmBufferStats {
    /// DMA fill now, %.
    pub dma_fill_pct: u8,
    /// DMA lowest fill, %.
    pub dma_min_pct: u8,
    /// DMA highest fill, %.
    pub dma_max_pct: u8,
    /// Ring fill now, %.
    pub ring_fill_pct: u8,
    /// Ring lowest fill, %.
    pub ring_min_pct: u8,
    /// Ring highest fill, %.
    pub ring_max_pct: u8,
}

/// REQ_GET_BUFFER_STATS (44 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BufferStats {
    /// Valid entries of `spdif`.
    pub num_spdif: u8,
    /// The PDM output is active.
    pub pdm_active: bool,
    /// USB audio is streaming.
    pub streaming: bool,
    /// Increments with each snapshot.
    pub sequence: u16,
    /// Per S/PDIF output.
    pub spdif: [SpdifBufferStats; 4],
    /// The PDM output.
    pub pdm: PdmBufferStats,
}

/// REQ_GET_SPDIF_RX_STATUS (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpdifRxStatus {
    /// 0 inactive, 1 acquiring, 2 locked, 3 relocking.
    pub state: u8,
    /// InputSource: 0 USB, 1 S/PDIF, 2 I2S, 3 ADAT, 4-6 S/PDIF 2-4.
    pub input_source: u8,
    /// Locks since start.
    pub lock_count: u8,
    /// Lock losses since start.
    pub loss_count: u8,
    /// Hz when locked (else the receiver's raw estimate or 0).
    pub sample_rate: u32,
    /// Parity errors since start.
    pub parity_errors: u32,
    /// Receive FIFO fill, %.
    pub fifo_fill_pct: u16,
    /// Receiver library debug: 0 no signal, 1 waiting stable, 2 stable.
    pub lib_state: u8,
    /// Stable callbacks of the receiver library (4 bits).
    pub stable_callbacks: u8,
    /// Lost callbacks of the receiver library (4 bits).
    pub lost_callbacks: u8,
}

// ── Conversion helpers ────────────────────────────────────────────────

/// `src` mapped into a fixed array: missing entries are `fill`, extra ones
/// are dropped.
fn arr<T: Copy, U, const N: usize>(src: &[U], fill: T, f: impl Fn(&U) -> T) -> [T; N] {
    let mut out = [fill; N];
    for (d, s) in out.iter_mut().zip(src) {
        *d = f(s);
    }
    out
}

/// `src` copied into a fixed array (see [`arr`]); missing entries are zero.
fn copy<T: Copy + Default, const N: usize>(src: &[T]) -> [T; N] {
    arr(src, T::default(), |x| *x)
}

/// A two-level `src` mapped into a fixed 2-D array (see [`arr`]).
fn arr2<T: Copy, U, const N: usize, const M: usize>(
    src: &[Vec<U>],
    fill: T,
    f: impl Fn(&U) -> T + Copy,
) -> [[T; M]; N] {
    let mut out = [[fill; M]; N];
    for (d, s) in out.iter_mut().zip(src) {
        *d = arr(s, fill, f);
    }
    out
}

/// `s` as a NUL-padded buffer: cut at a character boundary to at most
/// `N - 1` bytes, so the buffer always holds a terminating NUL.
fn name_buf<const N: usize>(s: &str) -> [u8; N] {
    let mut end = s.len().min(N.saturating_sub(1));
    while !s.is_char_boundary(end) {
        // Terminates: 0 is always a char boundary.
        end = end.saturating_sub(1);
    }
    let mut out = [0u8; N];
    for (d, b) in out.iter_mut().zip(s.as_bytes().iter().take(end)) {
        *d = *b;
    }
    out
}

/// The text of a NUL-padded buffer up to the first NUL; invalid UTF-8 is
/// replaced, never a panic.
fn buf_str(b: &[u8]) -> String {
    let len = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(b.get(..len).unwrap_or_default()).into_owned()
}

// ── Records -> C ──────────────────────────────────────────────────────

impl From<&r::FilterParams> for FilterParams {
    fn from(v: &r::FilterParams) -> Self {
        Self {
            filter_type: v.filter_type,
            bypass: v.bypass,
            freq: v.freq,
            q: v.q,
            gain: v.gain,
            qp: v.qp,
        }
    }
}

impl From<&r::InputLockStatus> for InputLockStatus {
    fn from(v: &r::InputLockStatus) -> Self {
        Self {
            state: v.state,
            clock_mode: v.clock_mode,
            detected_rate: v.detected_rate,
            measured_hz: v.measured_hz,
            rate_ok: v.rate_ok,
            lock_count: v.lock_count,
            loss_count: v.loss_count,
        }
    }
}

impl From<&r::AdatOutStatus> for AdatOutStatus {
    fn from(v: &r::AdatOutStatus) -> Self {
        Self {
            enabled: v.enabled,
            active: v.active,
            pin: v.pin,
            rate_ok: v.rate_ok,
            resync_count: v.resync_count,
            slip_count: v.slip_count,
        }
    }
}

impl From<&r::LgStatus> for LgStatus {
    fn from(v: &r::LgStatus) -> Self {
        Self {
            enabled: v.enabled,
            present: v.present,
            volume: v.volume,
            muted: v.muted,
        }
    }
}

impl From<&r::UartConfig> for UartConfig {
    fn from(v: &r::UartConfig) -> Self {
        Self {
            enabled: v.enabled,
            tx_pin: v.tx_pin,
            rx_pin: v.rx_pin,
            notify: v.notify,
            baud: v.baud,
        }
    }
}

impl From<&r::I2cConfig> for I2cConfig {
    fn from(v: &r::I2cConfig) -> Self {
        Self {
            enabled: v.enabled,
            sda_pin: v.sda_pin,
            scl_pin: v.scl_pin,
            address: v.address,
        }
    }
}

impl From<&r::CtrlIfaceStatus> for CtrlIfaceStatus {
    fn from(v: &r::CtrlIfaceStatus) -> Self {
        Self {
            uart_last_status: v.uart_last_status,
            uart_live: v.uart_live,
            i2c_last_status: v.i2c_last_status,
            i2c_live: v.i2c_live,
            protocol_version: v.protocol_version,
        }
    }
}

impl From<&r::UpmixStatus> for UpmixStatus {
    fn from(v: &r::UpmixStatus) -> Self {
        Self {
            active: v.active,
            parked_reason: v.parked_reason,
            correlation: v.correlation,
            balance: v.balance,
            center_gain: v.center_gain,
            ls_gain: v.ls_gain,
            rs_gain: v.rs_gain,
        }
    }
}

impl From<&r::SystemStatus> for SystemStatus {
    fn from(v: &r::SystemStatus) -> Self {
        Self {
            peaks: copy(&v.peaks),
            cpu0: v.cpu0,
            cpu1: v.cpu1,
            clip_flags: v.clip_flags,
            active_input_channels: v.active_input_channels,
            num_channels: v.num_channels,
        }
    }
}

impl From<&r::DeviceInfo> for DeviceInfo {
    /// The serial is cut to 63 bytes (at a character boundary) and
    /// NUL-terminated; `serial_len` counts the bytes before the NUL.
    fn from(v: &r::DeviceInfo) -> Self {
        let serial: [u8; 64] = name_buf(&v.serial);
        let len = serial.iter().position(|&c| c == 0).unwrap_or(0);
        Self {
            serial,
            // At most 63 (name_buf leaves a NUL), so the fallback is unreachable.
            serial_len: u32::try_from(len).unwrap_or(0),
            location_id: v.location_id,
        }
    }
}

impl From<&r::PresetDirectory> for PresetDirectory {
    fn from(v: &r::PresetDirectory) -> Self {
        Self {
            occupied_mask: v.occupied_mask,
            startup_mode: v.startup_mode,
            default_slot: v.default_slot,
            last_active: v.last_active,
            output_config_mode: v.output_config_mode,
            master_volume_mode: v.master_volume_mode,
        }
    }
}

impl From<&r::DspState> for DspState {
    /// Field for field. Vectors fill their arrays from index 0 (missing
    /// entries zero, extra ones dropped); names become NUL-padded buffers
    /// (at most 31 bytes, cut at a character boundary). The record's
    /// `crossfeed_pair_map` has no place in the old layout and is left out:
    /// the C ABI reaches it through the extension-channel functions (Q2b).
    fn from(v: &r::DspState) -> Self {
        let fp = FilterParams::default();
        let filt = |f: &r::FilterParams| FilterParams::from(f);
        Self {
            platform_id: v.platform_id,
            num_channels: v.num_channels,
            num_input_channels: v.num_input_channels,
            num_output_channels: v.num_output_channels,
            format_version: v.format_version,
            fw_major: v.fw_major,
            fw_minor: v.fw_minor,
            fw_patch: v.fw_patch,
            fw_beta: v.fw_beta,
            compat: v.compat,
            preamp_db: v.preamp_db,
            bypass: v.bypass,
            input_preamp_db: copy(&v.input_preamp_db),
            master_volume_db: v.master_volume_db,
            master_volume_mode: v.master_volume_mode,
            user_volume_db: v.user_volume_db,
            user_mute: v.user_mute,
            loudness_enabled: v.loudness_enabled,
            loudness_ref_spl: v.loudness_ref_spl,
            loudness_intensity: v.loudness_intensity,
            loudness_output_mask: v.loudness_output_mask,
            crossfeed_enabled: v.crossfeed_enabled,
            crossfeed_preset: v.crossfeed_preset,
            crossfeed_freq: v.crossfeed_freq,
            crossfeed_feed: v.crossfeed_feed,
            crossfeed_itd: v.crossfeed_itd,
            crossfeed_output_pair_mask: v.crossfeed_output_pair_mask,
            channel_delays: copy(&v.channel_delays),
            matrix_routing: arr2(&v.matrix_routing, false, |x| *x),
            matrix_gain: arr2(&v.matrix_gain, 0.0, |x| *x),
            matrix_invert: arr2(&v.matrix_invert, false, |x| *x),
            output_enabled: copy(&v.output_enabled),
            output_muted: copy(&v.output_muted),
            output_gain_db: copy(&v.output_gain_db),
            output_delay_ms: copy(&v.output_delay_ms),
            limiter_enabled: copy(&v.limiter_enabled),
            limiter_link_group: copy(&v.limiter_link_group),
            limiter_threshold_db: copy(&v.limiter_threshold_db),
            limiter_release_ms: copy(&v.limiter_release_ms),
            num_pin_outputs: v.num_pin_outputs,
            output_pins: copy(&v.output_pins),
            output_config_mode: v.output_config_mode,
            filters: arr2(&v.filters, fp, filt),
            xover: arr2(&v.xover, fp, filt),
            channel_names: arr(&v.channel_names, [0; CHANNEL_NAME_LEN], |s| name_buf(s)),
            leveller_enabled: v.leveller_enabled,
            leveller_speed: v.leveller_speed,
            leveller_lookahead: v.leveller_lookahead,
            leveller_amount: v.leveller_amount,
            leveller_max_gain_db: v.leveller_max_gain_db,
            leveller_gate_db: v.leveller_gate_db,
            leveller_detector_mask: v.leveller_detector_mask,
            leveller_apply_mask: v.leveller_apply_mask,
            input_source: v.input_source,
            lg_sound_sync_enabled: v.lg_sound_sync_enabled,
            spdif_inputs_enabled: v.spdif_inputs_enabled,
            adat_input_enabled: v.adat_input_enabled,
            spdif_rx_pins: copy(&v.spdif_rx_pins),
            i2s_rx_pins: copy(&v.i2s_rx_pins),
            i2s_input_channels: v.i2s_input_channels,
            i2s_input_rate: v.i2s_input_rate,
            i2s_clock_mode: v.i2s_clock_mode,
            adat_input_pin: v.adat_input_pin,
            adat_input_clock_mode: v.adat_input_clock_mode,
            psybass_enabled: v.psybass_enabled,
            psybass_output_mask: v.psybass_output_mask,
            psybass_cutoff_hz: v.psybass_cutoff_hz,
            psybass_harmonics_db: v.psybass_harmonics_db,
            psybass_drive_db: v.psybass_drive_db,
            psybass_character_pct: v.psybass_character_pct,
            psybass_original_db: v.psybass_original_db,
            upmix: copy(&v.upmix),
            subharm: copy(&v.subharm),
            subharm_solo: v.subharm_solo,
            tube: copy(&v.tube),
            output_types: copy(&v.output_types),
            i2s_bck_pin: v.i2s_bck_pin,
            i2s_bck_pin_slave: v.i2s_bck_pin_slave,
            i2s_clock_pin_mode: v.i2s_clock_pin_mode,
            mck_enabled: v.mck_enabled,
            mck_pin: v.mck_pin,
            mck_multiplier: v.mck_multiplier,
            adat_out_enabled: v.adat_out_enabled,
            adat_out_pin: v.adat_out_pin,
            dac_mute_supported: v.dac_mute_supported,
            dac_mute_enabled: v.dac_mute_enabled,
            dac_mute_active_low: v.dac_mute_active_low,
            dac_mute_pin: v.dac_mute_pin,
            dac_mute_hold_ms: v.dac_mute_hold_ms,
            dac_mute_release_ms: v.dac_mute_release_ms,
            ctrl_iface_supported: v.ctrl_iface_supported,
            uart: UartConfig::from(&v.uart),
            i2c: I2cConfig::from(&v.i2c),
            ctrl_status: CtrlIfaceStatus::from(&v.ctrl_status),
            core1_mode: v.core1_mode,
            preset_occupied: v.preset_occupied,
            preset_names: arr(&v.preset_names, [0; CHANNEL_NAME_LEN], |s| name_buf(s)),
            active_preset_slot: v.active_preset_slot,
            preset_startup_mode: v.preset_startup_mode,
            preset_default_slot: v.preset_default_slot,
            bulk_valid: v.bulk_valid,
            bulk_raw: copy(&v.bulk_raw),
        }
    }
}

impl From<&r::NotifyResult> for NotifyResult {
    fn from(v: &r::NotifyResult) -> Self {
        Self {
            flags: v.flags,
            filter_channels: v.filter_channels,
        }
    }
}

impl From<&r::RtaRequest> for RtaRequest {
    fn from(v: &r::RtaRequest) -> Self {
        Self {
            tap: v.tap,
            mask: v.mask,
            wants_bins: v.wants_bins,
            fft_order: v.fft_order,
            avg_ms: v.avg_ms,
            peak_decay_db_s: v.peak_decay_db_s,
        }
    }
}

impl From<&r::RtaCapsInfo> for RtaCapsInfo {
    fn from(v: &r::RtaCapsInfo) -> Self {
        Self {
            supported: v.supported,
            input_channels: v.input_channels,
            output_channels: v.output_channels,
            order_min: v.order_min,
            order_max: v.order_max,
            order_default: v.order_default,
            bass_bands: v.bass_bands,
            max_bands: v.max_bands,
            dynamic_range_db: v.dynamic_range_db,
            bass_dynamic_range_db: v.bass_dynamic_range_db,
            centres: copy(&v.centres),
        }
    }
}

impl From<&r::RtaChannelFrame> for RtaChannelFrame {
    /// Missing band levels read `RTA_FLOOR_DB`.
    fn from(v: &r::RtaChannelFrame) -> Self {
        Self {
            channel: v.channel,
            n_bands: v.n_bands,
            silent: v.silent,
            avg_db: arr(&v.avg_db, RTA_FLOOR_DB, |x| *x),
            peak_db: arr(&v.peak_db, RTA_FLOOR_DB, |x| *x),
        }
    }
}

impl From<&r::RtaSnapshot> for RtaSnapshot {
    /// Missing frames are silent ones, missing bins read `RTA_FLOOR_DB`.
    fn from(v: &r::RtaSnapshot) -> Self {
        Self {
            version: v.version,
            telemetry_version: v.telemetry_version,
            tap: v.tap,
            count: v.count,
            frames: arr(&v.frames, RtaChannelFrame::default(), |x| {
                RtaChannelFrame::from(x)
            }),
            populated: v.populated,
            bins_count: v.bins_count,
            bins_channel: v.bins_channel,
            bins_sample_rate: v.bins_sample_rate,
            bins_db: arr(&v.bins_db, RTA_FLOOR_DB, |x| *x),
            running: v.running,
            live_count: v.live_count,
            live_mask: v.live_mask,
            frames_per_s: v.frames_per_s,
            busy_us_per_s: v.busy_us_per_s,
            last_frame_us: v.last_frame_us,
            bass_busy_us_per_s: v.bass_busy_us_per_s,
            sample_rate: v.sample_rate,
            first_band: v.first_band,
            fft_order: v.fft_order,
            config_rejected: v.config_rejected,
            refresh_interval_s: v.refresh_interval_s,
        }
    }
}

impl From<&r::SiggenConfig> for SiggenConfig {
    fn from(v: &r::SiggenConfig) -> Self {
        Self {
            signal_type: v.signal_type,
            channel_mask: v.channel_mask,
            invert_mask: v.invert_mask,
            flags: v.flags,
            level_db: v.level_db,
            duration_ms: v.duration_ms,
            repeat: v.repeat,
            gap_ms: v.gap_ms,
            p: copy(&v.p),
        }
    }
}

impl From<&r::SiggenStatus> for SiggenStatus {
    fn from(v: &r::SiggenStatus) -> Self {
        Self {
            state: v.state,
            signal_type: v.signal_type,
            active_channel: v.active_channel,
            elapsed_ms: v.elapsed_ms,
            cycles_done: v.cycles_done,
            stop_reason: v.stop_reason,
            current_freq: v.current_freq,
        }
    }
}

impl From<&r::SiggenParam> for SiggenParam {
    fn from(v: &r::SiggenParam) -> Self {
        Self {
            semantic: v.semantic,
            min: v.min,
            max: v.max,
            def: v.def,
        }
    }
}

impl From<&r::SiggenTypeInfo> for SiggenTypeInfo {
    /// The name is cut to 8 bytes and NUL-terminated.
    fn from(v: &r::SiggenTypeInfo) -> Self {
        Self {
            id: v.id,
            name: name_buf(&v.name),
            timing: v.timing,
            params: arr(&v.params, SiggenParam::default(), |x| SiggenParam::from(x)),
        }
    }
}

impl From<&r::SiggenCaps> for SiggenCaps {
    fn from(v: &r::SiggenCaps) -> Self {
        Self {
            supported: v.supported,
            type_count: v.type_count,
            output_channels: v.output_channels,
            multitone_max: v.multitone_max,
            valid_mask: v.valid_mask,
            types: arr(&v.types, SiggenTypeInfo::default(), |x| {
                SiggenTypeInfo::from(x)
            }),
        }
    }
}

impl From<&r::SpdifBufferStats> for SpdifBufferStats {
    fn from(v: &r::SpdifBufferStats) -> Self {
        Self {
            free: v.free,
            prepared: v.prepared,
            playing: v.playing,
            fill_pct: v.fill_pct,
            min_pct: v.min_pct,
            max_pct: v.max_pct,
        }
    }
}

impl From<&r::PdmBufferStats> for PdmBufferStats {
    fn from(v: &r::PdmBufferStats) -> Self {
        Self {
            dma_fill_pct: v.dma_fill_pct,
            dma_min_pct: v.dma_min_pct,
            dma_max_pct: v.dma_max_pct,
            ring_fill_pct: v.ring_fill_pct,
            ring_min_pct: v.ring_min_pct,
            ring_max_pct: v.ring_max_pct,
        }
    }
}

impl From<&r::BufferStats> for BufferStats {
    fn from(v: &r::BufferStats) -> Self {
        Self {
            num_spdif: v.num_spdif,
            pdm_active: v.pdm_active,
            streaming: v.streaming,
            sequence: v.sequence,
            spdif: arr(&v.spdif, SpdifBufferStats::default(), |x| {
                SpdifBufferStats::from(x)
            }),
            pdm: PdmBufferStats::from(&v.pdm),
        }
    }
}

impl From<&r::SpdifRxStatus> for SpdifRxStatus {
    fn from(v: &r::SpdifRxStatus) -> Self {
        Self {
            state: v.state,
            input_source: v.input_source,
            lock_count: v.lock_count,
            loss_count: v.loss_count,
            sample_rate: v.sample_rate,
            parity_errors: v.parity_errors,
            fifo_fill_pct: v.fifo_fill_pct,
            lib_state: v.lib_state,
            stable_callbacks: v.stable_callbacks,
            lost_callbacks: v.lost_callbacks,
        }
    }
}

// ── C -> records (the records a C caller passes in) ───────────────────

impl From<&FilterParams> for r::FilterParams {
    fn from(v: &FilterParams) -> Self {
        Self {
            filter_type: v.filter_type,
            bypass: v.bypass,
            freq: v.freq,
            q: v.q,
            gain: v.gain,
            qp: v.qp,
        }
    }
}

impl From<&UartConfig> for r::UartConfig {
    fn from(v: &UartConfig) -> Self {
        Self {
            enabled: v.enabled,
            tx_pin: v.tx_pin,
            rx_pin: v.rx_pin,
            notify: v.notify,
            baud: v.baud,
        }
    }
}

impl From<&I2cConfig> for r::I2cConfig {
    fn from(v: &I2cConfig) -> Self {
        Self {
            enabled: v.enabled,
            sda_pin: v.sda_pin,
            scl_pin: v.scl_pin,
            address: v.address,
        }
    }
}

impl From<&RtaRequest> for r::RtaRequest {
    fn from(v: &RtaRequest) -> Self {
        Self {
            tap: v.tap,
            mask: v.mask,
            wants_bins: v.wants_bins,
            fft_order: v.fft_order,
            avg_ms: v.avg_ms,
            peak_decay_db_s: v.peak_decay_db_s,
        }
    }
}

impl From<&SiggenConfig> for r::SiggenConfig {
    fn from(v: &SiggenConfig) -> Self {
        Self {
            signal_type: v.signal_type,
            channel_mask: v.channel_mask,
            invert_mask: v.invert_mask,
            flags: v.flags,
            level_db: v.level_db,
            duration_ms: v.duration_ms,
            repeat: v.repeat,
            gap_ms: v.gap_ms,
            p: v.p.to_vec(),
        }
    }
}

impl From<&DeviceInfo> for r::DeviceInfo {
    /// `serial_len` is clamped to the buffer and stops at the first NUL;
    /// invalid UTF-8 is replaced.
    fn from(v: &DeviceInfo) -> Self {
        let len = usize::try_from(v.serial_len).unwrap_or(usize::MAX);
        let s = v.serial.get(..len.min(v.serial.len())).unwrap_or_default();
        Self {
            serial: buf_str(s),
            location_id: v.location_id,
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_q1;
