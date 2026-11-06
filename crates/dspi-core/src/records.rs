//! The records the embedder API hands out: plain Rust structs, binding
//! friendly (no fixed arrays, no pointers, no lifetimes).
//!
//! They started as the Android console's UniFFI records (`dspi-ffi`
//! `records.rs`), which mirror the old desktop core's `#[repr(C)]` structs
//! field for field: same names, same order. Fixed-size C arrays are `Vec`s of
//! the same length; NUL-padded name buffers are `String`s. Keeping that
//! mirror means the C ABI (`dspi-capi`) converts mechanically and the Kotlin
//! and Swift bindings (WP F1) see the field names the apps already use.
//!
//! Code values (filter types, `COMPAT_*`, `PRESET_*`, ...) stay raw integers,
//! as on the wire; the named constants are in `crate::consts`.

// ── types.rs ──────────────────────────────────────────────────────────

/// One filter band, PEQ or crossover (`types.rs` `FilterParams`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterParams {
    /// Firmware filter type (`FILTER_*`): 0..13 PEQ, 32..63 crossover.
    pub filter_type: u8,
    /// Whether the band is bypassed.
    pub bypass: bool,
    /// Centre or corner frequency, Hz.
    pub freq: f32,
    /// Q (dimensionless).
    pub q: f32,
    /// Gain in dB; for the Linkwitz Transform, the target frequency fp in Hz.
    pub gain: f32,
    /// Linkwitz Transform target Q (ignored by every other type).
    pub qp: f32,
}

/// Lock status of a clocked input (`fetch_input_lock`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct InputLockStatus {
    /// Receiver state, the firmware's raw code; 0 = inactive.
    pub state: u8,
    /// Clock mode, 0 master, 1 slave.
    pub clock_mode: u8,
    /// Sample rate the receiver decided on, Hz.
    pub detected_rate: u32,
    /// Sample rate as measured from the signal, Hz.
    pub measured_hz: u32,
    /// Whether the rate is usable; always true for the I2S slave.
    pub rate_ok: bool,
    /// Times it locked, wrapping 8-bit counter.
    pub lock_count: u8,
    /// Times it lost lock, wrapping 8-bit counter.
    pub loss_count: u8,
}

/// The ADAT output's status (`fetch_adat_out_status`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AdatOutStatus {
    /// The output is enabled in the configuration.
    pub enabled: bool,
    /// The output is running now.
    pub active: bool,
    /// GPIO of the output.
    pub pin: u8,
    /// The current sample rate suits ADAT.
    pub rate_ok: bool,
    /// Times the output resynchronised, wrapping counter.
    pub resync_count: u16,
    /// Sample slips, wrapping counter.
    pub slip_count: u16,
}

/// LG Sound Sync status (`fetch_lg_status`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LgStatus {
    /// LG Sound Sync is enabled.
    pub enabled: bool,
    /// A TV has been detected.
    pub present: bool,
    /// TV volume 0..100; 0xFF until one has been decoded.
    pub volume: u8,
    /// The TV reports mute.
    pub muted: bool,
}

/// The UART control interface's configuration (`set_uart`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UartConfig {
    /// The UART control interface is enabled.
    pub enabled: bool,
    /// TX GPIO (`pin % 4 == 0`).
    pub tx_pin: u8,
    /// RX GPIO (`pin % 4 == 1`), same UART instance.
    pub rx_pin: u8,
    /// Push notification frames over the UART.
    pub notify: bool,
    /// Baud rate, bits per second.
    pub baud: u32,
}

/// The I2C control interface's configuration (`set_i2c`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct I2cConfig {
    /// The I2C control interface is enabled.
    pub enabled: bool,
    /// SDA GPIO (even), same I2C instance as `scl_pin`.
    pub sda_pin: u8,
    /// SCL GPIO (odd).
    pub scl_pin: u8,
    /// 7-bit target address, 0x08..0x77.
    pub address: u8,
}

/// The control interfaces' status (`REQ_GET_CTRL_IFACE_STATUS`).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CtrlIfaceStatus {
    /// `PIN_CONFIG_*` result of the last UART config write.
    pub uart_last_status: u8,
    /// The UART interface is up now (a boot-time pin collision keeps it down).
    pub uart_live: bool,
    /// `PIN_CONFIG_*` result of the last I2C config write.
    pub i2c_last_status: u8,
    /// The I2C interface is up now.
    pub i2c_live: bool,
    /// The control protocol version the firmware speaks.
    pub protocol_version: u8,
}

/// The stereo upmixer's live telemetry (`fetch_upmix_status`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UpmixStatus {
    /// The upmixer is processing audio now.
    pub active: bool,
    /// 0 active, 1 disabled, 2 input not stereo, 3 sample rate above 48 kHz.
    pub parked_reason: u8,
    /// Smoothed L/R correlation, -1..1.
    pub correlation: f32,
    /// Smoothed L/R balance, 0..1.
    pub balance: f32,
    /// Smoothed centre extraction gain, linear 0..1.
    pub center_gain: f32,
    /// Left surround steering gain, linear 0..1.
    pub ls_gain: f32,
    /// Right surround steering gain, linear 0..1.
    pub rs_gain: f32,
}

/// `types.rs` `SystemStatus` (REQ_GET_STATUS wValue 9), polled every 60 ms.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemStatus {
    /// Peak per wire channel, 0..1 (MAX_CHANNELS = 17 entries).
    pub peaks: Vec<f32>,
    /// Core 0 load, %.
    pub cpu0: u8,
    /// Core 1 load, %.
    pub cpu1: u8,
    /// Sticky clip flags, bit per wire channel.
    pub clip_flags: u32,
    /// Input channels carrying the active source; 0 from firmware that does
    /// not report it.
    pub active_input_channels: u8,
    /// Wire channels the device has.
    pub num_channels: u8,
}

/// `types.rs` `DeviceInfo`; `serial` replaces the `serial`/`serial_len` pair.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceInfo {
    /// USB serial number, the picker's identity.
    pub serial: String,
    /// Where the unit sits on the bus: IOKit's location id on macOS, derived
    /// from the bus position elsewhere.
    pub location_id: u32,
}

/// The preset directory (`get_preset_directory`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PresetDirectory {
    /// Bit per slot: the slot holds a preset.
    pub occupied_mask: u16,
    /// `PRESET_STARTUP_*`: 0 load `default_slot`, 1 load the last active.
    pub startup_mode: u8,
    /// The slot loaded at boot in mode 0.
    pub default_slot: u8,
    /// The slot last loaded or saved.
    pub last_active: u8,
    /// `OUTPUT_CONFIG_MODE_*`: 0 stored independently, 1 with the preset.
    pub output_config_mode: u8,
    /// `MASTER_VOLUME_MODE_*`: 0 stored independently, 1 with the preset.
    pub master_volume_mode: u8,
}

// ── state.rs ──────────────────────────────────────────────────────────

/// Full mirror of `state.rs` `DspState` (wire format V32).
///
/// Channel-indexed vectors use wire channel indices (`[inputs][outputs]`,
/// MAX_CHANNELS = 17 entries); output-indexed ones have MAX_OUTPUTS = 9,
/// input-indexed MAX_INPUTS = 8. Only the first `num_*` entries are live.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DspState {
    // Device / firmware
    /// 0 RP2040, 1 RP2350.
    pub platform_id: u8,
    /// Live wire channels (inputs + outputs).
    pub num_channels: u8,
    /// Live input channels.
    pub num_input_channels: u8,
    /// Live output channels.
    pub num_output_channels: u8,
    /// The device's own wire format version.
    pub format_version: u8,
    /// Firmware version, major.
    pub fw_major: u8,
    /// Firmware version, minor.
    pub fw_minor: u8,
    /// Firmware version, patch.
    pub fw_patch: u8,
    /// Beta number, 0 = final; `FW_BETA_EARLY` for an early beta that does not say.
    pub fw_beta: u8,
    /// `COMPAT_*` (0 unknown, 1 ok, 2 firmware too old, 3 too new).
    pub compat: u8,
    // Global
    /// Preamp, dB.
    pub preamp_db: f32,
    /// Master EQ bypass.
    pub bypass: bool,
    /// Per-input preamp, dB (input-indexed).
    pub input_preamp_db: Vec<f32>,
    // Volume
    /// Master volume, dB.
    pub master_volume_db: f32,
    /// `MASTER_VOLUME_MODE_*`: 0 stored independently, 1 with the preset.
    pub master_volume_mode: u8,
    /// User volume, dB.
    pub user_volume_db: f32,
    /// User mute.
    pub user_mute: bool,
    // Loudness
    /// Loudness compensation on.
    pub loudness_enabled: bool,
    /// Loudness reference level, dB SPL.
    pub loudness_ref_spl: f32,
    /// Loudness intensity, %.
    pub loudness_intensity: f32,
    /// Bit k: loudness processes output k.
    pub loudness_output_mask: u16,
    // Crossfeed
    /// Crossfeed on.
    pub crossfeed_enabled: bool,
    /// Crossfeed preset index.
    pub crossfeed_preset: u8,
    /// Custom crossfeed frequency, Hz.
    pub crossfeed_freq: f32,
    /// Custom crossfeed feed level, dB.
    pub crossfeed_feed: f32,
    /// Interaural time delay on.
    pub crossfeed_itd: bool,
    /// Bit p: crossfeed processes stereo output pair p.
    pub crossfeed_output_pair_mask: u8,
    /// Outputs of each crossfeed pair, 4 bytes (K340 firmware fork; zero
    /// from other firmware). Byte p: 0 = default (2p, 2p+1), else
    /// `(right << 4) | left`, 0-based output indices.
    pub crossfeed_pair_map: Vec<u8>,
    // Delays (wire channel)
    /// Delay per wire channel, ms.
    pub channel_delays: Vec<f32>,
    // Matrix [input][output]
    /// Crosspoint on.
    pub matrix_routing: Vec<Vec<bool>>,
    /// Crosspoint gain, dB.
    pub matrix_gain: Vec<Vec<f32>>,
    /// Crosspoint polarity inverted.
    pub matrix_invert: Vec<Vec<bool>>,
    // Outputs
    /// Output enabled.
    pub output_enabled: Vec<bool>,
    /// Output muted.
    pub output_muted: Vec<bool>,
    /// Output gain, dB.
    pub output_gain_db: Vec<f32>,
    /// Output delay, ms.
    pub output_delay_ms: Vec<f32>,
    // Limiter
    /// Limiter on, per output.
    pub limiter_enabled: Vec<bool>,
    /// Limiter link group, per output; outputs in one group limit together.
    pub limiter_link_group: Vec<u8>,
    /// Limiter threshold, dB, per output.
    pub limiter_threshold_db: Vec<f32>,
    /// Limiter release, ms, per output.
    pub limiter_release_ms: Vec<f32>,
    // Pins
    /// Physical output slots: 3 on RP2040, 5 on RP2350.
    pub num_pin_outputs: u8,
    /// GPIO of each physical output slot.
    pub output_pins: Vec<u8>,
    /// `OUTPUT_CONFIG_MODE_*`: 0 stored independently, 1 with the preset.
    pub output_config_mode: u8,
    // EQ [wire channel][band]
    /// PEQ bands, `[wire channel][band]`.
    pub filters: Vec<Vec<FilterParams>>,
    /// Crossover bands, `[wire channel][band]`.
    pub xover: Vec<Vec<FilterParams>>,
    // Names (wire channel)
    /// Channel names (wire channel), at most 31 bytes of UTF-8 each.
    pub channel_names: Vec<String>,
    // Leveller
    /// Leveller on.
    pub leveller_enabled: bool,
    /// Leveller speed: 0 slow, 1 medium, 2 fast.
    pub leveller_speed: u8,
    /// Leveller lookahead on.
    pub leveller_lookahead: bool,
    /// Leveller amount, %.
    pub leveller_amount: f32,
    /// Leveller maximum gain, dB.
    pub leveller_max_gain_db: f32,
    /// Leveller gate threshold, dB.
    pub leveller_gate_db: f32,
    /// Bit k: input channel k feeds the leveller's detector.
    pub leveller_detector_mask: u8,
    /// Bit k: the leveller's gain applies to input channel k.
    pub leveller_apply_mask: u8,
    // Input / misc
    /// Active input (`InputSource`: 0 USB, 1 S/PDIF, 2 I2S, ...).
    pub input_source: u8,
    /// LG Sound Sync on.
    pub lg_sound_sync_enabled: bool,
    /// Bit k: S/PDIF input k+1 is enabled. Bit 0 (input 1) is always set:
    /// the first S/PDIF input cannot be disabled, and setters must keep the
    /// bit when they write the mask.
    pub spdif_inputs_enabled: u8,
    /// ADAT input enabled.
    pub adat_input_enabled: bool,
    /// GPIO of each S/PDIF receiver.
    pub spdif_rx_pins: Vec<u8>,
    /// I2S data GPIO of each stereo input pair.
    pub i2s_rx_pins: Vec<u8>,
    /// Active I2S input channels: 2, 4, 6 or 8.
    pub i2s_input_channels: u8,
    /// I2S input rate code: 0 44.1 kHz, 1 48 kHz, 2 96 kHz.
    pub i2s_input_rate: u8,
    /// I2S clock: 0 master, 1 slave.
    pub i2s_clock_mode: u8,
    /// ADAT input GPIO.
    pub adat_input_pin: u8,
    /// ADAT input clock: 0 master, 1 slave.
    pub adat_input_clock_mode: u8,
    // Psychoacoustic bass
    /// Psychoacoustic bass on.
    pub psybass_enabled: bool,
    /// Bit k: psychoacoustic bass processes output k.
    pub psybass_output_mask: u16,
    /// Psychoacoustic bass cutoff, Hz.
    pub psybass_cutoff_hz: f32,
    /// Harmonics level, dB.
    pub psybass_harmonics_db: f32,
    /// Drive, dB.
    pub psybass_drive_db: f32,
    /// Character, %.
    pub psybass_character_pct: f32,
    /// Level of the original bass, dB.
    pub psybass_original_db: f32,
    // Upmixer / subharm / tube, by *_PARAM_* id
    /// Upmixer parameters, by `UPMIX_PARAM_*`.
    pub upmix: Vec<f32>,
    /// Subharmonic synthesizer parameters, by `SUBHARM_PARAM_*`.
    pub subharm: Vec<f32>,
    /// Subharmonic solo (runtime only, not in the image).
    pub subharm_solo: bool,
    /// Tube preamp parameters, by `TUBE_PARAM_*`.
    pub tube: Vec<f32>,
    // Output types and clocks
    /// Type of each physical output slot: `OUTPUT_TYPE_*` (0 S/PDIF, 1 I2S).
    pub output_types: Vec<u8>,
    /// I2S BCK GPIO as clock master.
    pub i2s_bck_pin: u8,
    /// I2S BCK GPIO as clock slave.
    pub i2s_bck_pin_slave: u8,
    /// I2S clock pins: 0 unified, 1 split.
    pub i2s_clock_pin_mode: u8,
    /// I2S MCK output on.
    pub mck_enabled: bool,
    /// MCK GPIO.
    pub mck_pin: u8,
    /// MCK multiplier: 128 or 256.
    pub mck_multiplier: u8,
    /// ADAT output enabled.
    pub adat_out_enabled: bool,
    /// ADAT output GPIO.
    pub adat_out_pin: u8,
    // DAC hardware mute
    /// The firmware has the DAC hardware mute.
    pub dac_mute_supported: bool,
    /// DAC hardware mute on.
    pub dac_mute_enabled: bool,
    /// Pull the pin low to mute.
    pub dac_mute_active_low: bool,
    /// DAC mute GPIO.
    pub dac_mute_pin: u8,
    /// Hold after the mute asserts, before the clock stops, ms.
    pub dac_mute_hold_ms: u16,
    /// Hold after the clock restarts, before the unmute, ms.
    pub dac_mute_release_ms: u16,
    // Control interfaces
    /// The firmware has the UART and I2C control interfaces.
    pub ctrl_iface_supported: bool,
    /// UART control interface configuration.
    pub uart: UartConfig,
    /// I2C control interface configuration.
    pub i2c: I2cConfig,
    /// Control interfaces' status.
    pub ctrl_status: CtrlIfaceStatus,
    // Core 1
    /// `CORE1_MODE_*`: 0 idle, 1 PDM, 2 EQ worker.
    pub core1_mode: u8,
    // Presets
    /// Bit per slot: the slot holds a preset.
    pub preset_occupied: u16,
    /// Preset names, per slot.
    pub preset_names: Vec<String>,
    /// The active preset slot.
    pub active_preset_slot: u8,
    /// `PRESET_STARTUP_*`: 0 load `preset_default_slot`, 1 load the last active.
    pub preset_startup_mode: u8,
    /// The slot loaded at boot in mode 0.
    pub preset_default_slot: u8,
    // Raw bulk image (BULK_PARAMS_SIZE = 6136 bytes)
    /// The image is in this build's wire format, so the mirror may be written back.
    pub bulk_valid: bool,
    /// The image itself.
    pub bulk_raw: Vec<u8>,
}

// ── notify.rs ─────────────────────────────────────────────────────────

/// What `process_notifications` changed: `NOTIFY_*` flags and a bitmask of
/// wire channels whose filters changed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NotifyResult {
    /// `NOTIFY_*` flags of what changed.
    pub flags: u32,
    /// Bit per wire channel: its filters changed.
    pub filter_channels: u32,
}

// ── rta.rs ────────────────────────────────────────────────────────────

/// What the analyser should measure (`rta_set_request`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RtaRequest {
    /// `RTA_TAP_INPUT` (0) / `RTA_TAP_OUTPUT` (1).
    pub tap: u8,
    /// Channel mask within the tap; 0 releases the analyser.
    pub mask: u16,
    /// Ask for the FFT bins as well as the bands; the device sends them for a
    /// single analysed channel.
    pub wants_bins: bool,
    /// FFT order, within the caps' `order_min..=order_max`.
    pub fft_order: u8,
    /// Averaging time, ms; 0 = none beyond the detector.
    pub avg_ms: u16,
    /// Peak decay, dB/s; 0 = peak hold off.
    pub peak_decay_db_s: u8,
}

/// The analyser's capabilities (`rta_get_caps`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaCapsInfo {
    /// The firmware has an analyser.
    pub supported: bool,
    /// Input channels it can tap.
    pub input_channels: u8,
    /// Output channels it can tap.
    pub output_channels: u8,
    /// Lowest FFT order.
    pub order_min: u8,
    /// Highest FFT order.
    pub order_max: u8,
    /// Default FFT order.
    pub order_default: u8,
    /// Bands of the continuous bass bank, from band 0.
    pub bass_bands: u8,
    /// Bands in a frame.
    pub max_bands: u8,
    /// Usable range of the FFT bands, dB.
    pub dynamic_range_db: u8,
    /// Usable range of the bass bands, dB.
    pub bass_dynamic_range_db: u16,
    /// Band centre frequencies, RTA_MAX_BANDS = 37 entries.
    pub centres: Vec<u16>,
}

/// One analysed channel's bands.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaChannelFrame {
    /// Channel within the tap.
    pub channel: u8,
    /// Bands that carry data.
    pub n_bands: u8,
    /// No frame arrived for this channel.
    pub silent: bool,
    /// Average level per band, dBFS.
    pub avg_db: Vec<f32>,
    /// Peak level per band, dBFS.
    pub peak_db: Vec<f32>,
}

/// The analyser's latest frames and status (`rta_get_snapshot`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaSnapshot {
    /// Changes when something drawn changed (`rta_get_snapshot`).
    pub version: u32,
    /// Changes when the status line changed.
    pub telemetry_version: u32,
    /// `RTA_TAP_INPUT` (0) / `RTA_TAP_OUTPUT` (1).
    pub tap: u8,
    /// Channels analysed.
    pub count: u8,
    /// RTA_MAX_CHANNELS = 16 entries; the first `count` are the analysed
    /// channels (packed, ascending channel), the rest are defaults.
    pub frames: Vec<RtaChannelFrame>,
    /// Bands that carry a level (`rta::populated_bands`): bit b = band b
    /// (RTA_MAX_BANDS = 37); the bass bank always, an FFT band only when a bin
    /// centre falls inside its edges, none below `first_band`.
    pub populated: u64,
    /// Valid entries of `bins_db`.
    pub bins_count: u16,
    /// Channel the bins belong to.
    pub bins_channel: u8,
    /// Sample rate the bins were computed at, Hz.
    pub bins_sample_rate: u32,
    /// First `bins_count` of RTA_MAX_BINS = 512 entries are valid.
    pub bins_db: Vec<f32>,
    /// The analyser is running on the device.
    pub running: bool,
    /// Channels the device reports live.
    pub live_count: u8,
    /// Bit per channel of the tap: live on the device.
    pub live_mask: u16,
    /// Transforms per second.
    pub frames_per_s: u16,
    /// Main-loop transform time, µs per second.
    pub busy_us_per_s: u16,
    /// Wall time of the last complete transform, µs.
    pub last_frame_us: u16,
    /// Bass bank time on both cores, µs per second, saturating.
    pub bass_busy_us_per_s: u16,
    /// Current sample rate, Hz.
    pub sample_rate: u32,
    /// First band this FFT resolves; 0xFF = unknown.
    pub first_band: u8,
    /// FFT order in force.
    pub fft_order: u8,
    /// The device refused the last requested configuration.
    pub config_rejected: bool,
    /// How long one channel waits between frames, s.
    pub refresh_interval_s: f32,
}

// ── siggen.rs ─────────────────────────────────────────────────────────

/// A signal generator configuration (`siggen_set_config`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenConfig {
    /// `SIGGEN_TYPE_*`.
    pub signal_type: u8,
    /// Bit per output channel: the signal plays there.
    pub channel_mask: u16,
    /// The subset of `channel_mask` with inverted polarity.
    pub invert_mask: u16,
    /// `SIGGEN_FLAG_*`.
    pub flags: u8,
    /// Peak level, dBFS, -120..0.
    pub level_db: f32,
    /// Duration, ms.
    pub duration_ms: u32,
    /// Repeat count.
    pub repeat: u16,
    /// Gap between repeats, ms.
    pub gap_ms: u16,
    /// Type-specific parameters, exactly 4.
    pub p: Vec<f32>,
}

/// What the generator is doing (`siggen_get_status`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenStatus {
    /// `SIGGEN_STATE_*`: idle, fade in, run, gap, fade out.
    pub state: u8,
    /// The active or last `SIGGEN_TYPE_*`.
    pub signal_type: u8,
    /// The walked output; 0xFF when not walking.
    pub active_channel: u8,
    /// Time since start, ms.
    pub elapsed_ms: u32,
    /// Completed sweeps or pattern periods.
    pub cycles_done: u16,
    /// `SIGGEN_STOP_*` of the last stop.
    pub stop_reason: u8,
    /// Instantaneous sweep frequency, Hz; 0 when not sweeping.
    pub current_freq: f32,
}

/// One parameter of a signal type.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenParam {
    /// `SIGGEN_PARAM_*`: what the value means (0 = unused).
    pub semantic: u8,
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// Default value.
    pub def: f32,
}

/// One signal type the generator offers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenTypeInfo {
    /// `SIGGEN_TYPE_*`; 0xFF when its descriptor could not be read.
    pub id: u8,
    /// Display name.
    pub name: String,
    /// `SIGGEN_TIMING_*`: continuous, sweep or pattern.
    pub timing: u8,
    /// Exactly 4.
    pub params: Vec<SiggenParam>,
}

/// The generator's capabilities (`siggen_fetch_caps`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenCaps {
    /// The firmware has a generator.
    pub supported: bool,
    /// Signal types it reports.
    pub type_count: u8,
    /// Output channels it can drive.
    pub output_channels: u8,
    /// Most tones of a multitone signal.
    pub multitone_max: u8,
    /// Bit per channel: valid in `SiggenConfig.channel_mask`.
    pub valid_mask: u16,
    /// SIGGEN_TYPE_COUNT = 15 entries; first `type_count` valid.
    pub types: Vec<SiggenTypeInfo>,
}

// ── stats.rs ──────────────────────────────────────────────────────────

/// One S/PDIF output's buffers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpdifBufferStats {
    /// Free ring space, 48-frame buckets, 0..16.
    pub free: u8,
    /// Ring fill, 48-frame buckets, 0..16.
    pub prepared: u8,
    /// DMA buffers in flight, 0..1.
    pub playing: u8,
    /// Consumer pipeline fill, %.
    pub fill_pct: u8,
    /// Lowest fill since the last reset, %.
    pub min_pct: u8,
    /// Highest fill since the last reset, %.
    pub max_pct: u8,
}

/// The PDM output's buffers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PdmBufferStats {
    /// DMA buffer fill, %.
    pub dma_fill_pct: u8,
    /// Lowest DMA fill since the last reset, %.
    pub dma_min_pct: u8,
    /// Highest DMA fill since the last reset, %.
    pub dma_max_pct: u8,
    /// Software ring fill, %.
    pub ring_fill_pct: u8,
    /// Lowest ring fill since the last reset, %.
    pub ring_min_pct: u8,
    /// Highest ring fill since the last reset, %.
    pub ring_max_pct: u8,
}

/// Buffer fill levels and watermarks (`fetch_buffer_stats`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BufferStats {
    /// Live entries of `spdif`, at most 4.
    pub num_spdif: u8,
    /// The PDM output is active.
    pub pdm_active: bool,
    /// Audio is streaming.
    pub streaming: bool,
    /// Monotonic counter, wrapping.
    pub sequence: u16,
    /// Exactly 4.
    pub spdif: Vec<SpdifBufferStats>,
    /// The PDM output.
    pub pdm: PdmBufferStats,
}

/// The S/PDIF receiver (`fetch_spdif_rx_status`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpdifRxStatus {
    /// 0 inactive, 1 acquiring, 2 locked, 3 relocking.
    pub state: u8,
    /// The input source, raw `InputSource`.
    pub input_source: u8,
    /// Times it locked.
    pub lock_count: u8,
    /// Times it lost lock.
    pub loss_count: u8,
    /// Sample rate when locked, Hz.
    pub sample_rate: u32,
    /// Parity errors counted.
    pub parity_errors: u32,
    /// Receive FIFO fill, %.
    pub fifo_fill_pct: u16,
    /// The receiver library's debug state.
    pub lib_state: u8,
    /// Stable callbacks, 4-bit counter.
    pub stable_callbacks: u8,
    /// Lost callbacks, 4-bit counter.
    pub lost_callbacks: u8,
}

// ── monitor.rs (typed replacement of the C ABI's TSV text) ───────────

/// One line of the notification monitor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorEntry {
    /// Rising id, for `monitor_read`.
    pub id: u64,
    /// Host time, ms since the Unix epoch.
    pub time_ms: i64,
    /// The entry, for display.
    pub text: String,
}

/// Result of `monitor_read(after_id)`: the entries after `after_id` and the
/// id to pass next time (`after_id` itself if there were none).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorBatch {
    /// The id to pass next time.
    pub last_id: u64,
    /// Entries after `after_id`, oldest first.
    pub entries: Vec<MonitorEntry>,
}

// ── Glue-only records ────────────────────────────────────────────────

/// One GPIO held by a live control-surface binding (`dspi_cs_pin_uses`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PinUse {
    /// The GPIO.
    pub pin: u8,
    /// The binding's slot.
    pub slot: u8,
}

/// `dspi_bootloader_counts`. Always 0/0 on Android (libusb cannot
/// enumerate there); Android uses the Kotlin BootloaderLocator instead.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BootloaderCounts {
    /// RP2040 boards in BOOTSEL mode.
    pub rp2040: u32,
    /// RP2350 boards in BOOTSEL mode.
    pub rp2350: u32,
}

/// Version facts of the linked core, for the About page and sanity checks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoreInfo {
    /// dspi-ffi crate version.
    pub ffi_version: String,
    /// `protocol::WIRE_FORMAT_VERSION` (32).
    pub wire_format_version: u8,
    /// `protocol::MIN_WIRE_FORMAT_VERSION` (16).
    pub min_wire_format_version: u8,
    /// `protocol::BULK_PARAMS_SIZE` (6136).
    pub bulk_params_size: u32,
    /// Number of points in `compute_magnitude_curve` / `compute_phase_curve` (201).
    pub magnitude_points: u32,
}

// ── DSPi Link (dspi-link crate; DSPi-Link spec 1.2) ───────────────────

/// The hub's `hello` after `link_connect` (spec 7.2).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkHello {
    /// `hid`: lowercase UUID, stable per hub installation. Tokens are keyed on it.
    pub hub_id: String,
    /// The hub's display name.
    pub hub_name: String,
    /// `console` (DSPi Console in gateway mode) or `bridge` (ESP32 on the UART).
    pub hub_kind: String,
    /// The hub's software version.
    pub hub_version: String,
    /// The Link protocol minor version it speaks.
    pub proto_minor: u32,
    /// `none` or `pin`.
    pub auth: String,
    /// `window` (PIN shown on the hub) or `fixed` (the hub's own PIN); `window` when absent.
    pub pin_source: String,
    /// True: ask the user for the PIN and call `link_pair`.
    pub needs_pairing: bool,
    /// `cmd`, `notify`, `poll`, `snapshot`, `lock`, `fw_install`, `rename`, ...
    pub caps: Vec<String>,
    /// Largest payload it takes, bytes.
    pub max_payload: u32,
    /// Requests it allows in flight.
    pub max_inflight: u32,
}

/// One device the hub shares (`device.list`, spec 7.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkDevice {
    /// The hub's handle for it, for `link_select_device`.
    pub handle: u32,
    /// USB serial number.
    pub serial: String,
    /// The board's own name, else the hub's.
    pub name: String,
    /// `major.minor.patch`, "" when the hub did not say.
    pub fw: String,
    /// Beta number, 0 = final, -1 = unknown.
    pub fw_beta: i32,
    /// 0 RP2040, 1 RP2350, -1 unknown.
    pub platform: i32,
    /// `online`, `offline`, `updating`.
    pub state: String,
    /// `usb` or `uart` (latency and bulk timing).
    pub link: String,
    /// Session holding the device lock, 0 = none.
    pub locked_by: u32,
}

/// A `[bRequest, dir]` row of `policy.denied` (dir 0 = SET, 1 = GET).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LinkDenied {
    /// The `bRequest`.
    pub request: u8,
    /// 0 = SET, 1 = GET.
    pub dir: u8,
}

/// Where the current Link session stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkStatus {
    /// A session exists (connected, maybe not yet authenticated).
    pub connected: bool,
    /// Authenticated: commands and the device list work.
    pub ready: bool,
    /// Our session id (the NOTIFY `origin` of our own writes), 0 = unknown.
    pub session: u32,
    /// `viewer`, `control` or `admin`.
    pub role: String,
    /// Commands this role may not send (`policy.denied`).
    pub denied: Vec<LinkDenied>,
    /// The session ended (see `close_code` / `close_reason`).
    pub closed: bool,
    /// The hub's close code, -1 = none.
    pub close_code: i32,
    /// The hub's close reason.
    pub close_reason: String,
}

/// `GET /dspi/v1/info` (spec 3.2), for a hub added by address.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkInfo {
    /// Lowercase UUID, stable per hub installation.
    pub hub_id: String,
    /// The hub's display name.
    pub hub_name: String,
    /// `console` or `bridge`.
    pub hub_kind: String,
    /// `none` or `pin`.
    pub auth: String,
    /// The Link protocol minor version it speaks.
    pub proto_minor: u32,
    /// WebSocket path (`/dspi/v1` by default).
    pub ws_path: String,
    /// Serials of the devices it shares.
    pub serials: Vec<String>,
}

/// A typed hub address (spec 3.3).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkAddress {
    /// Host name or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

// ── Added by the core merge (docs/core-merge/PLAN.md) ────────────────────

/// The extension channel (Bofr5555/DSPi fork; `REQ_EXTENSION` 0x00), as
/// probed on connect (WP S3) and read on demand (WP C8).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtStatus {
    /// `EXT_LIST`: supported sub-command ids; empty on stock firmware.
    pub supported: Vec<u16>,
    /// Whether `EXT_CROSSFEED_PAIR_MAP` is usable: listed by `EXT_LIST` and
    /// read successfully on connect.
    pub has_pair_map: bool,
    /// Whether `EXT_CORE_VREG` is usable: listed by `EXT_LIST` and read
    /// successfully on connect.
    pub has_core_vreg: bool,
}

/// `EXT_CORE_VREG` status (16 bytes, `dspi_proto::ext::CoreVregStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreVregInfo {
    /// Live VREG code; `millivolts` is its voltage (0 if unknown).
    pub live_vreg: u8,
    /// Core voltage, mV; 0 if unknown.
    pub millivolts: u16,
    /// 0 = build default.
    pub stored_sel: u8,
    /// The build's default VREG code.
    pub default_vreg: u8,
    /// The firmware raised the voltage by itself after hangs.
    pub auto_raised: bool,
    /// The automatic raise is disabled.
    pub auto_disabled: bool,
    /// Hangs counted.
    pub hang_count: u8,
    /// A voltage trial is running.
    pub trial_active: bool,
    /// The trial's VREG code.
    pub trial_vreg: u8,
    /// VREG code of the last failed trial.
    pub failed_trial_vreg: u8,
    /// Uptime when that trial failed, s.
    pub failed_trial_uptime_s: u32,
    /// Uptime, s.
    pub uptime_s: u32,
}

/// `REQ_OUTPUT_LATENCY` status (config.h; 12 bytes). Field meanings as in
/// the firmware's `OutputLatencyStatus`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLatency {
    /// The raw 12-byte reply, for fields this build does not name yet.
    pub raw: Vec<u8>,
    /// Decoded `(name, value)` pairs, in the firmware's order.
    pub fields: Vec<NamedValue>,
}

/// `REQ_GET_CLOCK_DIAG` (0x82): empty unless the firmware was built with
/// `DSPI_CLOCK_DIAG`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockDiag {
    /// The firmware was built with `DSPI_CLOCK_DIAG`.
    pub available: bool,
    /// The raw reply.
    pub raw: Vec<u8>,
    /// Decoded `(name, value)` pairs, in the firmware's order.
    pub fields: Vec<NamedValue>,
}

/// A decoded diagnostic value.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedValue {
    /// The value's name.
    pub name: String,
    /// The value, as text.
    pub value: String,
}

/// What a `.uf2` file is (WP C11): checked before anything is flashed.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Uf2Info {
    /// Whether the file is a well-formed UF2 for an RP2040 or RP2350.
    pub valid: bool,
    /// Why not, when `valid` is false.
    pub problem: String,
    /// UF2 family id (`0xE48BFF56` RP2040, `0xE48BFF59` RP2350 ARM-S, ...).
    pub family_id: u32,
    /// 0 RP2040, 1 RP2350, -1 unknown.
    pub platform: i8,
    /// UF2 blocks in the file.
    pub num_blocks: u32,
    /// Bytes of flash the image covers.
    pub payload_bytes: u32,
    /// The DSPi firmware version found in the image (`1.1.6`), "" if none.
    /// It is the image's program-version string, so an untagged build
    /// carries a `git describe` suffix (`1.1.6-beta3-16-g557bce7`); compare
    /// versions on the leading `major.minor.patch[-betaN]` only.
    pub firmware_version: String,
}

/// A mounted RP2040/RP2350 bootloader drive (`RPI-RP2` / `RP2350`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootloaderDrive {
    /// Mount point or drive root.
    pub path: String,
    /// 0 RP2040, 1 RP2350, -1 unknown (from `INFO_UF2.TXT`).
    pub platform: i8,
    /// `INFO_UF2.TXT`, verbatim.
    pub info: String,
}

// Work-package sections: append new records only inside your own section
// (docs/core-merge/WP-BRIEF.md). Keep the order.
// -- WP C0 --
// -- WP C1 --
// -- WP C1b --
// -- WP C2 --
// -- WP C3 --
// -- WP C4 --
// -- WP C5 --
// -- WP C6 --
// -- WP C7 --
// -- WP C8 --

/// One output-latency option (`OutputLatencyOptionWire`, config.h; 8
/// bytes), from `fetch_output_latency_options` (WP C8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLatencyOption {
    /// `OutputLatencyId`, the value `set_output_latency` takes (never 0).
    pub id: u8,
    /// `OUTPUT_LATENCY_FLAG_STREAM_ONLY`: runs only with a streamed input
    /// (S/PDIF, I2S, ADAT); USB input runs the fallback instead.
    pub stream_only: bool,
    /// `OUTPUT_LATENCY_WIRE_AVAILABLE`: runs as itself on the active source.
    pub available: bool,
    /// Nominal ring fill target, µs.
    pub latency_us: u16,
    /// Nominal stream-input batch, µs.
    pub batch_us: u16,
    /// What choosing it gives now (active source and rate applied), µs.
    pub effective_us: u16,
}
// -- WP C9 --
// -- WP C10 --
// -- WP C11 --
// -- WP C12 --
// -- WP C13 --
// -- WP C14 --
// -- WP C15 --
// -- WP F1 --
