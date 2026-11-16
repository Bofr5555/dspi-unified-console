//! The plain data the core hands to its embedders.
//!
//! Every type here is a value: no handles, no locks, no behaviour beyond the
//! derives. The same structs cross every boundary the core has: UniFFI
//! records for Kotlin and Swift (feature `uniffi`), JSON for the web and
//! Link clients (`serde::Serialize`, camelCase keys, on the types that need
//! it), and the C ABI, which converts them field by field in `dspi-capi`.
//!
//! Field order is part of that contract (the Kotlin and Swift constructors
//! take the fields positionally), so new fields go at the end of a struct.

// ---------------------------------------------------------------- filters

/// One filter band as the device stores it: a PEQ band or a crossover slot.
///
/// The meaning of `gain` and `qp` depends on the type: a Linkwitz Transform
/// (type 11) carries its target corner `fp` in `gain` and its target Q in
/// `qp`; every other type ignores `qp`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterParams {
    /// The firmware's filter type byte (`FILTER_*`): 0 flat, 1..13 the PEQ
    /// shapes, 32..63 the crossover family/order/pass codes.
    pub filter_type: u8,
    /// True when the band is switched off without losing its settings.
    pub bypass: bool,
    /// Centre or corner frequency in Hz (`f0` for a Linkwitz Transform).
    pub freq: f32,
    /// Quality factor (`Q0` for a Linkwitz Transform).
    pub q: f32,
    /// Gain in dB; the target corner `fp` in Hz for a Linkwitz Transform.
    pub gain: f32,
    /// The Linkwitz Transform's target Q; unused by the other types.
    pub qp: f32,
}

// ------------------------------------------------- inputs and interfaces

/// Lock state of a clocked digital input (S/PDIF, ADAT or I2S slave).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct InputLockStatus {
    /// The receiver's state code as the firmware reports it.
    pub state: u8,
    /// Which clock the input runs on (the firmware's clock-mode code).
    pub clock_mode: u8,
    /// The sample rate the receiver decoded, in Hz (0 when unknown).
    pub detected_rate: u32,
    /// The measured incoming rate, in Hz.
    pub measured_hz: u32,
    /// True when the incoming rate is one the device can run at.
    pub rate_ok: bool,
    /// How often the input has locked since boot (wrapping counter).
    pub lock_count: u8,
    /// How often the input has lost lock since boot (wrapping counter).
    pub loss_count: u8,
}

/// State of the ADAT output.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AdatOutStatus {
    /// The output is configured on.
    pub enabled: bool,
    /// The output is actually sending frames.
    pub active: bool,
    /// The GPIO the output drives.
    pub pin: u8,
    /// True when the current sample rate is one ADAT can carry.
    pub rate_ok: bool,
    /// Resynchronisations since boot.
    pub resync_count: u16,
    /// Frame slips since boot.
    pub slip_count: u16,
}

/// State of the LG Sound Sync link (optical volume from an LG TV).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LgStatus {
    /// Sound Sync handling is switched on.
    pub enabled: bool,
    /// A TV is sending Sound Sync data.
    pub present: bool,
    /// The volume the TV last sent.
    pub volume: u8,
    /// The TV's mute flag.
    pub muted: bool,
}

/// Settings of the UART control interface.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UartConfig {
    /// The interface is switched on.
    pub enabled: bool,
    /// The transmit GPIO.
    pub tx_pin: u8,
    /// The receive GPIO.
    pub rx_pin: u8,
    /// The device sends unsolicited change notifications over the UART.
    pub notify: bool,
    /// Line speed in baud.
    pub baud: u32,
}

/// Settings of the I2C control interface.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct I2cConfig {
    /// The interface is switched on.
    pub enabled: bool,
    /// The data GPIO.
    pub sda_pin: u8,
    /// The clock GPIO.
    pub scl_pin: u8,
    /// The device's 7-bit address on the bus.
    pub address: u8,
}

/// Live state of the UART and I2C control interfaces.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CtrlIfaceStatus {
    /// Status code of the last UART command.
    pub uart_last_status: u8,
    /// The UART has seen traffic recently.
    pub uart_live: bool,
    /// Status code of the last I2C command.
    pub i2c_last_status: u8,
    /// The I2C bus has seen traffic recently.
    pub i2c_live: bool,
    /// Version of the control-interface protocol the firmware speaks.
    pub protocol_version: u8,
}

/// Live state of the stereo-to-surround upmixer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UpmixStatus {
    /// The upmixer is processing audio.
    pub active: bool,
    /// Why it is parked when it is not active (the firmware's reason code).
    pub parked_reason: u8,
    /// Measured left/right correlation.
    pub correlation: f32,
    /// Measured left/right balance.
    pub balance: f32,
    /// Current gain of the derived centre channel.
    pub center_gain: f32,
    /// Current gain of the left surround channel.
    pub ls_gain: f32,
    /// Current gain of the right surround channel.
    pub rs_gain: f32,
}

// ------------------------------------------------------- device and state

/// Levels and load, polled for the meters.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemStatus {
    /// Peak level per channel, linear full scale, inputs first.
    pub peaks: Vec<f32>,
    /// Load of core 0 in percent.
    pub cpu0: u8,
    /// Load of core 1 in percent.
    pub cpu1: u8,
    /// One bit per channel that clipped since the last poll.
    pub clip_flags: u32,
    /// Input channels the host is currently streaming.
    pub active_input_channels: u8,
    /// Number of entries in `peaks` that belong to the device.
    pub num_channels: u8,
    /// True when these values are a held copy because the last poll failed.
    pub stale: bool,
    /// Age of the values in milliseconds.
    pub age_ms: u32,
    // -- WP P0M --
    /// The held peak per channel (17 entries), linear: a new peak holds for
    /// `MeterBallistics.hold_ms`, then falls at `decay_per_s`. Computed by the
    /// core from `peaks`; draw it as the hold marker.
    pub peak_hold: Vec<f32>,
    /// The highest peak per channel (17 entries) since the connection opened
    /// or `reset_meter_max`.
    pub peak_max: Vec<f32>,
    /// The clip hold, one bit per channel: lit by a clip, dark again
    /// `MeterBallistics.clip_hold_ms` after the last new one (the core then
    /// clears the device's sticky `clip_flags`; a channel still clipping lights
    /// again on the next read) or after `clear_clips`. Where the clear is
    /// refused (a viewer on a hub) it follows the sticky flags. Draw the clip
    /// marker from this, not from `clip_flags`.
    pub clip_hold: u32,
}

/// A DSPi found on USB.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceInfo {
    /// The USB serial number string.
    pub serial: String,
    /// The bus location, to tell two devices with one serial apart.
    pub location_id: u32,
}

/// The device's preset slots and startup settings.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PresetDirectory {
    /// One bit per slot that holds a preset.
    pub occupied_mask: u16,
    /// What the device loads at power-up (`PRESET_STARTUP_*`).
    pub startup_mode: u8,
    /// The slot loaded at power-up in the "specified" startup mode.
    pub default_slot: u8,
    /// The slot loaded most recently.
    pub last_active: u8,
    /// Whether a preset load also applies the output configuration.
    pub output_config_mode: u8,
    /// Whether a preset load also applies the master volume.
    pub master_volume_mode: u8,
}

/// Everything the core knows about the connected device's settings.
///
/// Per-channel vectors are sized to the protocol maxima, not to the device:
/// `num_input_channels` and `num_output_channels` say how much of each is
/// real. Channels are numbered inputs first, then outputs.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DspState {
    /// The board's platform code (RP2040, RP2350, ...).
    pub platform_id: u8,
    /// Inputs plus outputs on this device.
    pub num_channels: u8,
    /// Input channels on this device.
    pub num_input_channels: u8,
    /// Output channels on this device.
    pub num_output_channels: u8,
    /// Wire format version of the bulk parameter image.
    pub format_version: u8,
    /// Firmware major version.
    pub fw_major: u8,
    /// Firmware minor version.
    pub fw_minor: u8,
    /// Firmware patch version.
    pub fw_patch: u8,
    /// Firmware beta number (0 for a release).
    pub fw_beta: u8,
    /// How this build relates to the firmware (`COMPAT_*`).
    pub compat: u8,
    /// Global preamp in dB.
    pub preamp_db: f32,
    /// The whole DSP chain is bypassed.
    pub bypass: bool,
    /// Preamp per input, in dB.
    pub input_preamp_db: Vec<f32>,
    /// Master volume in dB.
    pub master_volume_db: f32,
    /// How the master volume behaves (fixed, saved, ...): firmware code.
    pub master_volume_mode: u8,
    /// The user volume in dB.
    pub user_volume_db: f32,
    /// The user mute.
    pub user_mute: bool,
    /// Loudness compensation is on.
    pub loudness_enabled: bool,
    /// Loudness reference level in dB SPL.
    pub loudness_ref_spl: f32,
    /// Loudness intensity in percent.
    pub loudness_intensity: f32,
    /// Outputs loudness applies to, one bit each.
    pub loudness_output_mask: u16,
    /// Crossfeed is on.
    pub crossfeed_enabled: bool,
    /// Crossfeed preset number.
    pub crossfeed_preset: u8,
    /// Crossfeed corner frequency in Hz (custom preset).
    pub crossfeed_freq: f32,
    /// Crossfeed level in dB (custom preset).
    pub crossfeed_feed: f32,
    /// Interaural time delay is on.
    pub crossfeed_itd: bool,
    /// Output pairs crossfeed applies to, one bit each.
    pub crossfeed_output_pair_mask: u8,
    /// Which outputs each crossfeed pair uses (`EXT_CROSSFEED_PAIR_MAP`).
    pub crossfeed_pair_map: Vec<u8>,
    /// Per-channel delay in ms.
    pub channel_delays: Vec<f32>,
    /// Matrix routing, `[input][output]`.
    pub matrix_routing: Vec<Vec<bool>>,
    /// Matrix gain in dB, `[input][output]`.
    pub matrix_gain: Vec<Vec<f32>>,
    /// Matrix polarity inversion, `[input][output]`.
    pub matrix_invert: Vec<Vec<bool>>,
    /// Output enabled, per output.
    pub output_enabled: Vec<bool>,
    /// Output muted, per output.
    pub output_muted: Vec<bool>,
    /// Output gain in dB, per output.
    pub output_gain_db: Vec<f32>,
    /// Output delay in ms, per output.
    pub output_delay_ms: Vec<f32>,
    /// Limiter on, per output.
    pub limiter_enabled: Vec<bool>,
    /// Limiter link group, per output.
    pub limiter_link_group: Vec<u8>,
    /// Limiter threshold in dB, per output.
    pub limiter_threshold_db: Vec<f32>,
    /// Limiter release in ms, per output.
    pub limiter_release_ms: Vec<f32>,
    /// Physical output pins the device has.
    pub num_pin_outputs: u8,
    /// GPIO per physical output.
    pub output_pins: Vec<u8>,
    /// Output configuration mode (firmware code).
    pub output_config_mode: u8,
    /// PEQ bands, `[channel][band]`.
    pub filters: Vec<Vec<FilterParams>>,
    /// Crossover slots, `[channel][slot]`.
    pub xover: Vec<Vec<FilterParams>>,
    /// User names of the channels.
    pub channel_names: Vec<String>,
    /// Volume leveller is on.
    pub leveller_enabled: bool,
    /// Leveller speed (firmware code).
    pub leveller_speed: u8,
    /// Leveller lookahead is on.
    pub leveller_lookahead: bool,
    /// Leveller amount in percent.
    pub leveller_amount: f32,
    /// Leveller maximum gain in dB.
    pub leveller_max_gain_db: f32,
    /// Leveller gate in dB.
    pub leveller_gate_db: f32,
    /// Channels the leveller listens to, one bit each.
    pub leveller_detector_mask: u8,
    /// Channels the leveller acts on, one bit each.
    pub leveller_apply_mask: u8,
    /// The selected audio input (USB, S/PDIF, ...).
    pub input_source: u8,
    /// LG Sound Sync is on.
    pub lg_sound_sync_enabled: bool,
    /// How many S/PDIF inputs are enabled.
    pub spdif_inputs_enabled: u8,
    /// The ADAT input is enabled.
    pub adat_input_enabled: bool,
    /// GPIO per S/PDIF receiver.
    pub spdif_rx_pins: Vec<u8>,
    /// GPIO per I2S data input.
    pub i2s_rx_pins: Vec<u8>,
    /// Channels on the I2S input.
    pub i2s_input_channels: u8,
    /// I2S input rate (firmware code).
    pub i2s_input_rate: u8,
    /// I2S clock mode (master or slave).
    pub i2s_clock_mode: u8,
    /// GPIO of the ADAT input.
    pub adat_input_pin: u8,
    /// ADAT input clock mode.
    pub adat_input_clock_mode: u8,
    /// Psychoacoustic bass is on.
    pub psybass_enabled: bool,
    /// Outputs psychoacoustic bass applies to, one bit each.
    pub psybass_output_mask: u16,
    /// Psychoacoustic bass cutoff in Hz.
    pub psybass_cutoff_hz: f32,
    /// Harmonics level in dB.
    pub psybass_harmonics_db: f32,
    /// Drive in dB.
    pub psybass_drive_db: f32,
    /// Even/odd character in percent.
    pub psybass_character_pct: f32,
    /// Level of the original low band in dB.
    pub psybass_original_db: f32,
    /// Upmixer parameters, indexed by `UPMIX_PARAM_*`.
    pub upmix: Vec<f32>,
    /// Subharmonic synth parameters, indexed by `SUBHARM_PARAM_*`.
    pub subharm: Vec<f32>,
    /// The subharmonic synth's solo switch.
    pub subharm_solo: bool,
    /// Tube modeller parameters, indexed by the firmware's tube ids.
    pub tube: Vec<f32>,
    /// Kind of each output pin (S/PDIF, I2S, PDM, ...).
    pub output_types: Vec<u8>,
    /// I2S bit clock GPIO in master mode.
    pub i2s_bck_pin: u8,
    /// I2S bit clock GPIO in slave mode.
    pub i2s_bck_pin_slave: u8,
    /// How the I2S clock pins are assigned.
    pub i2s_clock_pin_mode: u8,
    /// The master clock output is on.
    pub mck_enabled: bool,
    /// GPIO of the master clock.
    pub mck_pin: u8,
    /// Master clock as a multiple of the sample rate.
    pub mck_multiplier: u8,
    /// The ADAT output is on.
    pub adat_out_enabled: bool,
    /// GPIO of the ADAT output.
    pub adat_out_pin: u8,
    /// The firmware has the DAC mute line.
    pub dac_mute_supported: bool,
    /// The DAC mute line is on.
    pub dac_mute_enabled: bool,
    /// The DAC mute line is active low.
    pub dac_mute_active_low: bool,
    /// GPIO of the DAC mute line.
    pub dac_mute_pin: u8,
    /// How long the mute is held, in ms.
    pub dac_mute_hold_ms: u16,
    /// How long the release takes, in ms.
    pub dac_mute_release_ms: u16,
    /// The firmware has the UART/I2C control interfaces.
    pub ctrl_iface_supported: bool,
    /// UART control interface settings.
    pub uart: UartConfig,
    /// I2C control interface settings.
    pub i2c: I2cConfig,
    /// Live state of the control interfaces.
    pub ctrl_status: CtrlIfaceStatus,
    /// What the second core runs (firmware code).
    pub core1_mode: u8,
    /// One bit per occupied preset slot.
    pub preset_occupied: u16,
    /// Name per preset slot.
    pub preset_names: Vec<String>,
    /// The slot loaded last.
    pub active_preset_slot: u8,
    /// Preset startup mode.
    pub preset_startup_mode: u8,
    /// Preset loaded at startup in the "specified" mode.
    pub preset_default_slot: u8,
    /// `bulk_raw` holds a parameter image this build can decode.
    pub bulk_valid: bool,
    /// The bulk parameter image as read from the device.
    pub bulk_raw: Vec<u8>,
    /// The image carried `i2s_clock_pin_mode` (newer wire formats).
    pub i2s_clock_pin_mode_present: bool,
    /// The image carried `spdif_inputs_enabled`.
    pub spdif_inputs_enabled_present: bool,
    /// The image carried `adat_input_enabled`.
    pub adat_input_enabled_present: bool,
    /// The image carried `adat_input_clock_mode`.
    pub adat_input_clock_mode_present: bool,
}

/// What a batch of device notifications changed.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NotifyResult {
    /// `NOTIFY_*` bits.
    pub flags: u32,
    /// One bit per channel whose curve must be redrawn.
    pub filter_channels: u32,
}

// --------------------------------------------------------------- analyser

/// What a view wants from the real-time analyser.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RtaRequest {
    /// Where to measure: `RTA_TAP_INPUT` or `RTA_TAP_OUTPUT`.
    pub tap: u8,
    /// Channels to measure, one bit each; 0 releases the analyser.
    pub mask: u16,
    /// Also fetch the raw FFT bins of one channel.
    pub wants_bins: bool,
    /// FFT size as a power of two.
    pub fft_order: u8,
    /// Averaging time in ms.
    pub avg_ms: u16,
    /// Peak hold decay in dB per second.
    pub peak_decay_db_s: u8,
    /// Show whatever another client is measuring instead of starting one.
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub watching: bool,
}

/// What the device's analyser can do.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaCapsInfo {
    /// The firmware has an analyser.
    pub supported: bool,
    /// Input channels it can measure.
    pub input_channels: u8,
    /// Output channels it can measure.
    pub output_channels: u8,
    /// Smallest FFT order.
    pub order_min: u8,
    /// Largest FFT order.
    pub order_max: u8,
    /// FFT order the device starts with.
    pub order_default: u8,
    /// Bands handled by the low-frequency filter bank.
    pub bass_bands: u8,
    /// Total bands.
    pub max_bands: u8,
    /// Dynamic range of the FFT bands in dB.
    pub dynamic_range_db: u8,
    /// Dynamic range of the bass bands in dB.
    pub bass_dynamic_range_db: u16,
    /// Centre frequency of each band in Hz.
    pub centres: Vec<u16>,
}

/// One channel's band levels in an analyser frame.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaChannelFrame {
    /// The channel these levels belong to.
    pub channel: u8,
    /// Valid entries in `avg_db` and `peak_db`.
    pub n_bands: u8,
    /// The channel carried no signal.
    pub silent: bool,
    /// Averaged level per band, dBFS.
    pub avg_db: Vec<f32>,
    /// Peak-hold level per band, dBFS.
    pub peak_db: Vec<f32>,
}

/// Everything the analyser view draws, as of the last poll.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RtaSnapshot {
    /// Bumped whenever the drawn levels change.
    pub version: u32,
    /// Bumped whenever the status fields change.
    pub telemetry_version: u32,
    /// The tap being measured.
    pub tap: u8,
    /// Valid entries in `frames`.
    pub count: u8,
    /// Levels per measured channel.
    pub frames: Vec<RtaChannelFrame>,
    /// One bit per band that has data at this rate and FFT size.
    pub populated: u64,
    /// Valid entries in `bins_db`.
    pub bins_count: u16,
    /// The channel the bins belong to.
    pub bins_channel: u8,
    /// Sample rate the bins were taken at, in Hz.
    pub bins_sample_rate: u32,
    /// Raw FFT bins in dBFS.
    pub bins_db: Vec<f32>,
    /// The analyser is running on the device.
    pub running: bool,
    /// Channels the device is measuring.
    pub live_count: u8,
    /// Mask of the channels the device is measuring.
    pub live_mask: u16,
    /// Analyser frames per second.
    pub frames_per_s: u16,
    /// Processor time spent per second, in µs.
    pub busy_us_per_s: u16,
    /// Time of the last frame, in µs.
    pub last_frame_us: u16,
    /// Processor time the bass bank spends per second, in µs.
    pub bass_busy_us_per_s: u16,
    /// Device sample rate in Hz.
    pub sample_rate: u32,
    /// First band with data; 0xFF when unknown.
    pub first_band: u8,
    /// FFT order in use.
    pub fft_order: u8,
    /// The device refused the requested configuration.
    pub config_rejected: bool,
    /// How often the view should poll, in seconds.
    pub refresh_interval_s: f32,
}

// ------------------------------------------------------- signal generator

/// A test signal to play.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenConfig {
    /// Signal type id.
    pub signal_type: u8,
    /// Channels to play on, one bit each.
    pub channel_mask: u16,
    /// Channels played inverted, one bit each.
    pub invert_mask: u16,
    /// `SIGGEN_FLAG_*` bits.
    pub flags: u8,
    /// Level in dBFS.
    pub level_db: f32,
    /// Length of one burst in ms (0: continuous).
    pub duration_ms: u32,
    /// Number of bursts.
    pub repeat: u16,
    /// Silence between bursts in ms.
    pub gap_ms: u16,
    /// The type's own parameters, as its `SiggenTypeInfo` describes them.
    pub p: Vec<f32>,
}

/// What the signal generator is doing.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenStatus {
    /// Generator state code.
    pub state: u8,
    /// Signal type playing.
    pub signal_type: u8,
    /// Channel playing (walk mode).
    pub active_channel: u8,
    /// Time since start in ms.
    pub elapsed_ms: u32,
    /// Bursts completed.
    pub cycles_done: u16,
    /// Why it last stopped.
    pub stop_reason: u8,
    /// Current frequency in Hz, for sweeps and tones.
    pub current_freq: f32,
}

/// One parameter slot of a signal type.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenParam {
    /// What the value means (frequency, ratio, ...): firmware code.
    pub semantic: u8,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// Default value.
    pub def: f32,
}

/// One signal type the generator offers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenTypeInfo {
    /// Type id.
    pub id: u8,
    /// Display name from the device.
    pub name: String,
    /// Timing class (continuous, burst, ...).
    pub timing: u8,
    /// Its parameter slots.
    pub params: Vec<SiggenParam>,
}

/// What the device's signal generator can do.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiggenCaps {
    /// The firmware has a signal generator.
    pub supported: bool,
    /// Signal types the device reports.
    pub type_count: u8,
    /// Output channels it can play on.
    pub output_channels: u8,
    /// Most tones in a multitone signal.
    pub multitone_max: u8,
    /// One bit per valid type id.
    pub valid_mask: u16,
    /// The types.
    pub types: Vec<SiggenTypeInfo>,
}

// ------------------------------------------------------ buffers and clocks

/// Fill of one S/PDIF output's buffers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdifBufferStats {
    /// Buffers free.
    pub free: u8,
    /// Buffers filled and waiting.
    pub prepared: u8,
    /// Buffers being played.
    pub playing: u8,
    /// Current fill in percent.
    pub fill_pct: u8,
    /// Lowest fill since the last read, percent.
    pub min_pct: u8,
    /// Highest fill since the last read, percent.
    pub max_pct: u8,
}

/// Fill of the PDM output's DMA and ring buffers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdmBufferStats {
    /// DMA buffer fill now, percent.
    pub dma_fill_pct: u8,
    /// Lowest DMA fill, percent.
    pub dma_min_pct: u8,
    /// Highest DMA fill, percent.
    pub dma_max_pct: u8,
    /// Ring buffer fill now, percent.
    pub ring_fill_pct: u8,
    /// Lowest ring fill, percent.
    pub ring_min_pct: u8,
    /// Highest ring fill, percent.
    pub ring_max_pct: u8,
}

/// Output buffer statistics.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferStats {
    /// Valid entries in `spdif`.
    pub num_spdif: u8,
    /// The PDM output is running.
    pub pdm_active: bool,
    /// Audio is streaming.
    pub streaming: bool,
    /// Counter of the device's statistics snapshot.
    pub sequence: u16,
    /// One entry per S/PDIF output.
    pub spdif: Vec<SpdifBufferStats>,
    /// The PDM output.
    pub pdm: PdmBufferStats,
}

/// State of the S/PDIF receiver.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpdifRxStatus {
    /// Receiver state code.
    pub state: u8,
    /// Input the receiver listens to.
    pub input_source: u8,
    /// Times locked since boot.
    pub lock_count: u8,
    /// Times lost since boot.
    pub loss_count: u8,
    /// Decoded sample rate in Hz.
    pub sample_rate: u32,
    /// Parity errors since boot.
    pub parity_errors: u32,
    /// Receive FIFO fill in percent.
    pub fifo_fill_pct: u16,
    /// State of the receiver library.
    pub lib_state: u8,
    /// Consecutive stable callbacks.
    pub stable_callbacks: u8,
    /// Callbacks lost.
    pub lost_callbacks: u8,
}

// ----------------------------------------------------------------- misc

/// One line of the traffic monitor.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorEntry {
    /// Increasing id.
    pub id: u64,
    /// Wall-clock time in ms since the Unix epoch.
    pub time_ms: i64,
    /// The line.
    pub text: String,
}

/// Monitor lines after a given id.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorBatch {
    /// The id to ask from next time.
    pub last_id: u64,
    /// The new lines, oldest first.
    pub entries: Vec<MonitorEntry>,
}

/// A GPIO used by a control-surface slot.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PinUse {
    /// The GPIO.
    pub pin: u8,
    /// The slot using it.
    pub slot: u8,
}

/// How many boards in bootloader mode are attached, per chip.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BootloaderCounts {
    /// RP2040 boards.
    pub rp2040: u32,
    /// RP2350 boards.
    pub rp2350: u32,
}

/// Version facts of this core build.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoreInfo {
    /// The crate version.
    pub ffi_version: String,
    /// Wire format version this build writes.
    pub wire_format_version: u8,
    /// Oldest wire format version this build reads.
    pub min_wire_format_version: u8,
    /// Size of the bulk parameter image in bytes.
    pub bulk_params_size: u32,
    /// Points on a response curve.
    pub magnitude_points: u32,
}

// ------------------------------------------------------------- DSPi Link

/// What a Link hub said about itself.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkHello {
    /// The hub's stable id.
    pub hub_id: String,
    /// The hub's display name.
    pub hub_name: String,
    /// What kind of program the hub is.
    pub hub_kind: String,
    /// The hub's version.
    pub hub_version: String,
    /// Minor version of the Link protocol.
    pub proto_minor: u32,
    /// Authentication the hub asks for.
    pub auth: String,
    /// Where the pairing PIN is shown.
    pub pin_source: String,
    /// The client has to pair first.
    pub needs_pairing: bool,
    /// The hub's capabilities.
    pub caps: Vec<String>,
    /// Largest message payload in bytes.
    pub max_payload: u32,
    /// Most requests in flight.
    pub max_inflight: u32,
    /// The stored token was refused.
    pub token_refused: bool,
}

/// A device a Link hub offers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkDevice {
    /// The hub's handle for the device.
    pub handle: u32,
    /// USB serial number.
    pub serial: String,
    /// Device name.
    pub name: String,
    /// Firmware version text.
    pub fw: String,
    /// Firmware beta number.
    pub fw_beta: i32,
    /// Platform code.
    pub platform: i32,
    /// Device state on the hub.
    pub state: String,
    /// How the hub reaches the device.
    pub link: String,
    /// Session holding the device's lock, 0 for none.
    pub locked_by: u32,
}

/// A request the hub refused for this session's role.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LinkDenied {
    /// The request code.
    pub request: u8,
    /// The direction: 0 for a SET, 1 for a GET.
    pub dir: u8,
}

/// State of the Link session.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkStatus {
    /// A connection to the hub exists.
    pub connected: bool,
    /// The session is bound to a device and usable.
    pub ready: bool,
    /// Session id given by the hub.
    pub session: u32,
    /// The role the hub granted.
    pub role: String,
    /// Requests refused for that role.
    pub denied: Vec<LinkDenied>,
    /// The hub closed the session.
    pub closed: bool,
    /// Close code.
    pub close_code: i32,
    /// Close reason.
    pub close_reason: String,
    /// Where the session is in its life.
    pub phase: LinkPhase,
    /// Reconnect attempt number.
    pub reconnect_attempt: u32,
    /// Time to the next reconnect in ms.
    pub reconnect_in_ms: u32,
    /// Where the pairing PIN is shown.
    pub pin_source: String,
    /// The stored token was refused.
    pub token_refused: bool,
}

/// A hub found on the network.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkInfo {
    /// The hub's stable id.
    pub hub_id: String,
    /// The hub's display name.
    pub hub_name: String,
    /// What kind of program the hub is.
    pub hub_kind: String,
    /// Authentication the hub asks for.
    pub auth: String,
    /// Minor version of the Link protocol.
    pub proto_minor: u32,
    /// WebSocket path.
    pub ws_path: String,
    /// Serials of the devices it offers.
    pub serials: Vec<String>,
}

/// A hub address.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkAddress {
    /// Host name or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

// -- WP SECCORE --
/// The current Link session's encryption (`link_security`; PLAN-SEC 4.3).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkSecurity {
    /// The session runs over TLS with a verified hub key. False: unencrypted
    /// (a hub without TLS, such as an upstream one): show the badge.
    pub encrypted: bool,
    // -- WP SECFIX --
    /// Not encrypted, but the hub is this machine (`localhost`, a loopback
    /// address: `dspi_link::address::is_loopback_host`): nothing crosses a
    /// network, so it is not shown as unsecured. Always false when
    /// `encrypted`. The three cases: `encrypted`, `local`, neither
    /// (unsecured: the red bar and " (unsecured)").
    pub local: bool,
    /// The hub key's fingerprint (`SHA256:K7Q2-M9XD-4TNB-0ZRA`), "" when none.
    pub hub_key: String,
    /// The last connection set a new pin (first use, or a rotation the old key
    /// signed): show "Encrypted connection set up; hub key ..." once.
    pub hub_key_new: bool,
    /// The credential to store for the hub ("" without a token): save it when it
    /// differs from the stored one.
    pub credential: String,
}

// -- WP SECCORE --
/// A parsed pairing link (`link_parse_pair_link`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, PartialEq, Default)]
pub struct LinkPairLink {
    /// The hub's id: `expected_hub_id`.
    pub hub_id: String,
    /// Where it answers.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// The `link_connect` URL.
    pub url: String,
    /// The fingerprint of the key it must present: show it on the pairing screen.
    pub hub_key: String,
    /// The `link_connect` credential: pinned to that key, no token yet.
    pub credential: String,
    /// The PIN to pair with after the user's tap ("" when the link has none).
    pub pin: String,
}

/// By hand: the PIN stays out of logs.
impl std::fmt::Debug for LinkPairLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkPairLink")
            .field("hub_id", &self.hub_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("hub_key", &self.hub_key)
            .field(
                "pin",
                &if self.pin.is_empty() {
                    ""
                } else {
                    "<redacted>"
                },
            )
            .finish_non_exhaustive()
    }
}

// ------------------------------------------------------ extension channel

/// What the extension channel supports.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtStatus {
    /// Extension ids the device lists.
    pub supported: Vec<u16>,
    /// The crossfeed pair map is supported.
    pub has_pair_map: bool,
    /// Core voltage control is supported.
    pub has_core_vreg: bool,
}

/// Core voltage regulator state.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreVregInfo {
    /// Regulator setting in use.
    pub live_vreg: u8,
    /// That setting in millivolts.
    pub millivolts: u16,
    /// Stored selection.
    pub stored_sel: u8,
    /// Factory setting.
    pub default_vreg: u8,
    /// The firmware raised the voltage on its own.
    pub auto_raised: bool,
    /// Automatic raising is off.
    pub auto_disabled: bool,
    /// Hangs counted.
    pub hang_count: u8,
    /// A trial setting is running.
    pub trial_active: bool,
    /// The trial setting.
    pub trial_vreg: u8,
    /// The last trial setting that failed.
    pub failed_trial_vreg: u8,
    /// Uptime at which it failed, in seconds.
    pub failed_trial_uptime_s: u32,
    /// Uptime in seconds.
    pub uptime_s: u32,
}

/// The output latency report.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLatency {
    /// The reply bytes.
    pub raw: Vec<u8>,
    /// The decoded fields.
    pub fields: Vec<NamedValue>,
}

/// The clock diagnostics report.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockDiag {
    /// The device answered.
    pub available: bool,
    /// The reply bytes.
    pub raw: Vec<u8>,
    /// The decoded fields.
    pub fields: Vec<NamedValue>,
}

/// A label and its formatted value.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedValue {
    /// The label.
    pub name: String,
    /// The value as text.
    pub value: String,
}

// --------------------------------------------------------------- firmware

/// What a UF2 firmware file holds.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Uf2Info {
    /// The file is a usable image.
    pub valid: bool,
    /// What is wrong with it, empty when valid.
    pub problem: String,
    /// UF2 family id.
    pub family_id: u32,
    /// Platform the image is for (-1 when no known family matches).
    pub platform: i8,
    /// UF2 blocks.
    pub num_blocks: u32,
    /// Payload bytes.
    pub payload_bytes: u32,
    /// Version string found in the image.
    pub firmware_version: String,
}

/// A mounted bootloader drive.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootloaderDrive {
    /// Mount path.
    pub path: String,
    /// Chip platform read from the drive (-1 when unknown).
    pub platform: i8,
    /// The drive's INFO_UF2.TXT text.
    pub info: String,
}

// ------------------------------------------------------- output latency

/// One output latency setting the device offers.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLatencyOption {
    /// Setting id.
    pub id: u8,
    /// Only possible while streaming from the host.
    pub stream_only: bool,
    /// Usable now.
    pub available: bool,
    /// Buffer latency in µs.
    pub latency_us: u16,
    /// Batch size in µs.
    pub batch_us: u16,
    /// Total latency in µs.
    pub effective_us: u16,
}

/// Where a Link session is in its life, for the status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum LinkPhase {
    /// No session.
    #[default]
    Idle,
    /// The hub wants a PIN.
    NeedsPairing,
    /// The hub has no device.
    NoDevice,
    /// The hub has several devices: pick one.
    ChooseDevice,
    /// Bound to a device.
    Bound,
    /// Waiting to reconnect.
    Reconnecting,
    /// Closed by the hub or the client.
    Closed,
    /// Refused by the hub.
    Refused,
}

// -------------------------------------------------- stream ring health

/// How the host's output stream ring has been doing.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RingHealth {
    /// Target fill in frames.
    pub target_frames: u16,
    /// Target fill in µs.
    pub target_us: u16,
    /// Frames per batch.
    pub batch_frames: u16,
    /// Underrun episodes.
    pub underrun_episodes: u32,
    /// Blocks the ring refused.
    pub refused_blocks: u32,
    /// Lowest fill seen, if any.
    pub lowest_fill: Option<u32>,
    /// Longest gap between input blocks, µs.
    pub worst_input_gap_us: u32,
    /// Longest gap between USB transfers, µs.
    pub worst_usb_gap_us: u32,
    /// Processing steps over budget.
    pub over_budget_steps: u32,
}

/// How healthy an output fill is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum FillBand {
    /// Comfortable.
    #[default]
    Good,
    /// Getting close to an edge.
    Warn,
    /// At or past an edge.
    Bad,
}

/// One output's fill.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFill {
    /// Fill now, percent.
    pub fill_pct: f64,
    /// Lowest fill, percent.
    pub min_pct: u8,
    /// Highest fill, percent.
    pub max_pct: u8,
    /// Its rating.
    pub band: FillBand,
}

/// One statistics poll.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTick {
    /// Buffer statistics.
    pub buffers: BufferStats,
    /// Per-output fills, when known.
    pub fills: Option<Vec<OutputFill>>,
}

/// Who runs the analyser on a shared device.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RtaOwnership {
    /// This client follows another one's configuration.
    pub following: bool,
    /// Who set the configuration.
    pub origin: u16,
    /// Its tap.
    pub tap: u8,
    /// Its channel mask.
    pub mask: u16,
    /// Its FFT order.
    pub fft_order: u8,
}

// -- WP P0M --
/// The meter ballistics' timing (`DspiCore::set_meter_ballistics`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterBallistics {
    /// How long a new peak holds, ms. Default 1000.
    pub hold_ms: u32,
    /// How fast a held peak falls after its hold, linear full scale per
    /// second. Default 0.5.
    pub decay_per_s: f32,
    /// How long the clip hold lasts after the last new clip, ms; 0 until
    /// cleared. Default 10 000 (the macOS console's).
    pub clip_hold_ms: u32,
}

/// One record of a device's event log (`DspiCore::event_log_read`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventEntry {
    /// Rising per device, across runs.
    pub id: u64,
    /// When, ms since the Unix epoch.
    pub time_ms: i64,
    /// `notification`, `connected`, `disconnected`, `clip`, `temperature`,
    /// `counter`, `reset`, `firmware`, `import`, `recall`; `unknown` for a
    /// newer build's kind.
    pub kind: String,
    /// The device's serial.
    pub serial: String,
    /// One readable line.
    pub text: String,
    /// Who or what caused it: a notification's source (`HOST`, `PRESET`,
    /// `UART`, ...), a transport, a counter, a reset reason, `meter`; "" when
    /// none.
    pub source: String,
    /// The DSPi Link session that caused it, when another one did.
    pub session: Option<u32>,
    /// This client's own write coming back.
    pub own: bool,
    /// A wire channel (clips), or the preset slot (recalls).
    pub channel: Option<u8>,
    /// °C for a temperature, the total for a counter.
    pub value: Option<f64>,
    /// The firmware version, on connects and firmware changes.
    pub fw: Option<String>,
    /// The device's uptime, s, where it reports one.
    pub uptime_s: Option<u32>,
    /// Detail lines: what a recall or an import changed.
    pub lines: Vec<String>,
    /// Occurrences this record stands for (repeats, counter steps, changes).
    pub count: u32,
}

/// A page of a device's event log.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventBatch {
    /// The id to pass next time.
    pub last_id: u64,
    /// Lines in the files that were not records.
    pub skipped: u32,
    /// Oldest first.
    pub entries: Vec<EventEntry>,
}

// -- WP P0E --
/// A file a client adds to a diagnostics bundle (`client/<name>` in the
/// zip): its own log, `uconsole doctor`'s report.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiagFile {
    /// The file's name; anything but letters, digits, `.`, `_` and `-`
    /// becomes `_`.
    pub name: String,
    /// Its text, kept up to 4 MiB.
    pub text: String,
}

/// What a client adds to a diagnostics bundle
/// (`DspiCore::diagnostics_save`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiagOptions {
    /// The client and its version, as it names itself ("uConsole for DSPi 1.2.0
    /// (Android)"); "" when it does not say.
    pub app: String,
    /// The folder of a hub store (`hub-store.json`) to include with its
    /// tokens and secrets removed; "" for none.
    pub hub_store_dir: String,
    /// The client's own files.
    pub files: Vec<DiagFile>,
}

// -- WP CS1 --
/// The device's USB error counters (`REQ_GET_USB_ERROR_STATS` 0xB2,
/// registry `diag.usb`): six `u32` since boot or the last reset
/// (`diag.usb.reset`). Current firmware reports them all as zero.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbErrorStats {
    /// Every error counted.
    pub total: u32,
    /// CRC errors.
    pub crc: u32,
    /// Bit-stuffing errors.
    pub bitstuff: u32,
    /// Receive overflows.
    pub rx_overflow: u32,
    /// Receive timeouts.
    pub rx_timeout: u32,
    /// Data sequence errors.
    pub data_seq: u32,
}

// -- WP P1AFIX --
/// The crossfeed's custom pair as the device keeps it
/// ([`crate::DspiCore::crossfeed_custom_pair`]): what Custom runs, whatever
/// preset is chosen now.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossfeedPair {
    /// Frequency, in Hz.
    pub freq_hz: f32,
    /// Feed level, in dB.
    pub feed_db: f32,
}

// -- WP CG-SWS-a --
/// One biquad section of a response curve ([`crate::curve_sections`]):
/// `H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)`, normalised so
/// that `a0` is 1, at the device's sample rate.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiquadSection {
    /// Feed-forward coefficient of `z^0`.
    pub b0: f64,
    /// Feed-forward coefficient of `z^-1`.
    pub b1: f64,
    /// Feed-forward coefficient of `z^-2`.
    pub b2: f64,
    /// Feedback coefficient of `z^-1`.
    pub a1: f64,
    /// Feedback coefficient of `z^-2`.
    pub a2: f64,
}

// -- WP CG-SWS-a --
/// What a platform is, as the clients lay out their pages
/// ([`crate::platform_shape`]): channel counts, where the PDM sub and the
/// second core's EQ outputs are, and which platform-bound sections exist.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformShape {
    /// The platform id the shape is for (0 RP2040, 1 RP2350, 2 STM32H723).
    pub platform_id: u8,
    /// `RP2040`, `RP2350`, `STM32H723`, or `unknown platform N`.
    pub name: String,
    /// False for an id this build does not know; its shape is the RP2040's,
    /// the smallest.
    pub known: bool,
    /// Matrix inputs (USB channels): 2 on RP2040, 8 elsewhere.
    pub num_input_channels: u8,
    /// Outputs, the PDM sub included: 5 on RP2040, 9 elsewhere.
    pub num_output_channels: u8,
    /// Stereo output slots (S/PDIF or I2S): 2 on RP2040, 4 elsewhere.
    pub num_output_slots: u8,
    /// The PDM sub's output index: always the last output.
    pub pdm_output: u8,
    /// The PDM sub's index in the output pin list: after the slots.
    pub pdm_pin_index: u8,
    /// First output whose EQ runs on the second core (the other half of the
    /// PDM interlock).
    pub eq_worker_first: u8,
    /// Last output whose EQ runs on the second core.
    pub eq_worker_last: u8,
    /// I2S input pairs: 1 on RP2040, 4 elsewhere.
    pub i2s_max_pairs: u8,
    /// Multichannel (more than one pair) I2S input: RP2350 only.
    pub multichannel_i2s: bool,
    /// ADAT output and input: RP2350 only.
    pub adat: bool,
    /// The stereo upmixer: RP2350 only (RP2040 stalls its writes).
    pub upmixer: bool,
    /// The I2S configuration page: every platform but STM32H723.
    pub i2s_config: bool,
    /// Crossfeed stereo pairs: 2 on RP2040, 4 elsewhere.
    pub crossfeed_pairs: u8,
    /// The longest delay the delay line holds, in ms: 42 on RP2350, 21
    /// elsewhere (`dspi_proto::Platform::max_delay_ms`).
    pub max_delay_ms: f32,
}

// -- WP CG-SWS-a --
/// Bytes of the parameter image that `DspState` has no field for
/// ([`crate::bulk_image_extras`]). A "present" flag is the firmware's
/// "0 is absent, keep the last value" byte, read on the V32 layout.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkImageExtras {
    /// The image is a whole one of this build's wire format (its first byte
    /// is the version): the mirror was decoded from it.
    pub decoded: bool,
    /// The image carried the I2S slave bit clock pin.
    pub i2s_bck_pin_slave_present: bool,
    /// The image carried the I2S input channel count.
    pub i2s_input_channels_present: bool,
    /// Per I2S data input 2..4: the image carried its pin.
    pub i2s_rx_pin_ext_present: Vec<bool>,
    /// Per S/PDIF input 2..4: the image carried its pin.
    pub spdif_rx_pin_ext_present: Vec<bool>,
    /// The image carried the ADAT output pin; without it the firmware uses
    /// its default pin.
    pub adat_out_pin_present: bool,
}

// -- WP CG-SWS-a --
/// One value of a choice setting and the words a client shows for it
/// ([`crate::enum_value_labels`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueLabel {
    /// The value as written to the setting.
    pub value: u8,
    /// The registry's name for the value (`passive`, `percussive`, ...), or
    /// the number for a numbered setting.
    pub key: String,
    /// What the Console shows for it.
    pub label: String,
}

// -- WP CG-SWS-a --
/// One key and value of a DNS-SD TXT record ([`crate::link_parse_txt`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTxtEntry {
    /// The key (`v`, `hid`, ...).
    pub key: String,
    /// Its value; empty for a key without one.
    pub value: String,
}

// -- WP CG-SWS-a --
/// A hub as its DNS-SD TXT record describes it ([`crate::link_parse_txt`]),
/// before an address is resolved.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTxtRecord {
    /// The `hid`, lowercase.
    pub hub_id: String,
    /// The instance name, safe to show (the `hid` when it has none).
    pub name: String,
    /// `console`, `bridge` or `unknown`.
    pub kind: String,
    /// `none` or `pin`.
    pub auth: String,
    /// Devices the hub shares.
    pub device_count: u32,
    /// Their serials, when the hub lists them.
    pub serials: Vec<String>,
    /// The WebSocket path.
    pub path: String,
    // -- WP SECDISC --
    /// The record carried `tls=1`: the hub says its port speaks TLS. A hint
    /// (anyone on the LAN can forge or strip it) for the connect's
    /// `tls_advertised`, the "advertised TLS never falls back" rule; false
    /// for an upstream hub.
    pub tls_advertised: bool,
}
