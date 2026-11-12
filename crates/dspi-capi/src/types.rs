//! The C ABI's records: `#[repr(C)]` structs with fixed arrays, as
//! `dspi_core.h` declares them, and their conversions from (and, for the
//! records a caller passes in, to) `dspi_core::records`.
//!
//! The binding-friendly records carry `Vec`s and `String`s; here each becomes
//! a fixed array of the header's size. Every conversion is total: a vector
//! shorter than its array leaves the rest at the field's neutral value (zero,
//! `false`, a default record, or the analyser floor for levels), a longer one
//! is cut, and a string is cut at a character boundary so that at least one
//! NUL byte ends it.
//!
//! The constants (`#define`s) are generated into `types/consts.rs`.

use std::ffi::{c_char, c_void};

use dspi_core::records as r;

mod consts;
pub use consts::*;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_q1;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1;

/// Hotplug callback (`dspi_set_hotplug_callback`): `event` 0 when a device
/// arrived; `serial` is valid during the call only.
pub type DeviceEventCallback =
    extern "C" fn(event: u8, serial: *const c_char, user_data: *mut c_void);

/// Wake-up callback (`dspi_set_notify_callback` and the analyser's): data is
/// waiting for the GUI thread.
pub type NotifyCallback = extern "C" fn(user_data: *mut c_void);

// ── Building fixed arrays ─────────────────────────────────────────────

/// `src` mapped through `f` into an array of `N`, `pad` where `src` runs out.
fn array_from<S, T: Copy, const N: usize>(src: &[S], pad: T, f: impl Fn(&S) -> T) -> [T; N] {
    let mut out = [pad; N];
    for (slot, item) in out.iter_mut().zip(src) {
        *slot = f(item);
    }
    out
}

/// `src` copied into an array of `N`, `pad` where it runs out.
fn copied<T: Copy, const N: usize>(src: &[T], pad: T) -> [T; N] {
    array_from(src, pad, |v| *v)
}

/// A `[outer][inner]` table from nested vectors, `pad` in every missing cell.
fn table<T: Copy, const I: usize, const O: usize>(src: &[Vec<T>], pad: T) -> [[T; I]; O] {
    array_from(src, [pad; I], |row| copied(row, pad))
}

/// `s` as a NUL-terminated byte buffer of `N`: at most `N - 1` bytes of it,
/// cut back to a character boundary, then zeros. An empty buffer stays empty.
pub(crate) fn name_buf<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let Some(room) = N.checked_sub(1) else {
        return out;
    };
    let cut = (0..=room.min(s.len()))
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    let bytes = s.as_bytes().get(..cut).unwrap_or_default();
    for (slot, b) in out.iter_mut().zip(bytes) {
        *slot = *b;
    }
    out
}

/// The text in a C byte buffer: up to `len` bytes and no further than the
/// first NUL, invalid UTF-8 replaced.
fn text_of(buf: &[u8], len: usize) -> String {
    let bytes = buf.get(..len.min(buf.len())).unwrap_or(buf);
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
}

// ── Records ───────────────────────────────────────────────────────────

/// One filter band, PEQ or crossover.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FilterParams {
    /// `FILTER_*`: 0..13 PEQ types, 32..63 crossover types.
    pub filter_type: u8,
    /// The band is bypassed.
    pub bypass: bool,
    /// Centre or corner frequency, Hz.
    pub freq: f32,
    /// Q.
    pub q: f32,
    /// Gain, dB; the Linkwitz Transform's target frequency, Hz.
    pub gain: f32,
    /// The Linkwitz Transform's target Q; unused by the other types.
    pub qp: f32,
}

/// Lock status of a clocked input.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputLockStatus {
    /// Receiver state as the firmware reports it; 0 inactive.
    pub state: u8,
    /// 0 clock master, 1 slave.
    pub clock_mode: u8,
    /// The rate the receiver settled on, Hz.
    pub detected_rate: u32,
    /// The rate measured on the signal, Hz.
    pub measured_hz: u32,
    /// The rate can be used.
    pub rate_ok: bool,
    /// Locks so far (wraps).
    pub lock_count: u8,
    /// Losses of lock so far (wraps).
    pub loss_count: u8,
}

/// The ADAT output's status.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AdatOutStatus {
    /// Enabled in the configuration.
    pub enabled: bool,
    /// Running now.
    pub active: bool,
    /// Its GPIO.
    pub pin: u8,
    /// The sample rate suits ADAT.
    pub rate_ok: bool,
    /// Resynchronisations (wraps).
    pub resync_count: u16,
    /// Sample slips (wraps).
    pub slip_count: u16,
}

/// LG Sound Sync status.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LgStatus {
    /// LG Sound Sync is on.
    pub enabled: bool,
    /// A TV was seen.
    pub present: bool,
    /// The TV's volume 0..100, 0xFF before the first one decoded.
    pub volume: u8,
    /// The TV reports mute.
    pub muted: bool,
}

/// The UART control interface's configuration.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UartConfig {
    /// The interface is on.
    pub enabled: bool,
    /// TX GPIO.
    pub tx_pin: u8,
    /// RX GPIO.
    pub rx_pin: u8,
    /// Notification frames go out over the UART.
    pub notify: bool,
    /// Bits per second.
    pub baud: u32,
}

/// The I2C control interface's configuration.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct I2cConfig {
    /// The interface is on.
    pub enabled: bool,
    /// SDA GPIO.
    pub sda_pin: u8,
    /// SCL GPIO.
    pub scl_pin: u8,
    /// 7-bit target address.
    pub address: u8,
}

/// The control interfaces' status.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CtrlIfaceStatus {
    /// `PIN_CONFIG_*` of the last UART configuration.
    pub uart_last_status: u8,
    /// The UART interface is up.
    pub uart_live: bool,
    /// `PIN_CONFIG_*` of the last I2C configuration.
    pub i2c_last_status: u8,
    /// The I2C interface is up.
    pub i2c_live: bool,
    /// The control protocol's version.
    pub protocol_version: u8,
}

/// The upmixer's live telemetry.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UpmixStatus {
    /// Processing audio now.
    pub active: bool,
    /// Why not: 0 active, 1 disabled, 2 input not stereo, 3 rate above 48 kHz.
    pub parked_reason: u8,
    /// L/R correlation, -1..1.
    pub correlation: f32,
    /// L/R balance, 0..1.
    pub balance: f32,
    /// Centre gain, linear.
    pub center_gain: f32,
    /// Left surround gain, linear.
    pub ls_gain: f32,
    /// Right surround gain, linear.
    pub rs_gain: f32,
}

/// Meters and load, polled.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SystemStatus {
    /// Peak per wire channel, 0..1.
    pub peaks: [f32; MAX_CHANNELS],
    /// Core 0 load, %.
    pub cpu0: u8,
    /// Core 1 load, %.
    pub cpu1: u8,
    /// Sticky clip flags, one bit per wire channel.
    pub clip_flags: u32,
    /// Input channels of the active source.
    pub active_input_channels: u8,
    /// Wire channels of the device.
    pub num_channels: u8,
    // -- WP P0M --
    /// Held peak per wire channel, 0..1: the hold marker (the core's
    /// ballistics, `dspi_set_meter_ballistics`).
    pub peak_hold: [f32; MAX_CHANNELS],
    /// Highest peak per wire channel since the connection opened or
    /// `dspi_reset_meter_max`.
    pub peak_max: [f32; MAX_CHANNELS],
    /// The clip hold, one bit per wire channel: draw the clip marker from
    /// this. The core clears the device's flags when it ends; a channel still
    /// clipping lights again, and where the clear is refused (a viewer on a
    /// hub) the hold follows the device's sticky flags.
    pub clip_hold: u32,
}

/// A device found by a scan.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceInfo {
    /// USB serial number, NUL-terminated.
    pub serial: [u8; 64],
    /// Its length in bytes, without the NUL.
    pub serial_len: u32,
    /// The unit's place on the bus.
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

/// Platform and firmware version.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformInfo {
    /// 0 RP2040, 1 RP2350.
    pub platform_id: u8,
    /// Output channels of the platform.
    pub num_output_channels: u8,
    /// Firmware major version.
    pub fw_major: u8,
    /// Firmware minor version.
    pub fw_minor: u8,
    /// Firmware patch version.
    pub fw_patch: u8,
    /// Firmware beta number, 0 for a release.
    pub fw_beta: u8,
}

/// The preset directory.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PresetDirectory {
    /// One bit per occupied slot.
    pub occupied_mask: u16,
    /// 0 load `default_slot` at boot, 1 the last active.
    pub startup_mode: u8,
    /// The slot loaded at boot in mode 0.
    pub default_slot: u8,
    /// The slot last loaded or saved.
    pub last_active: u8,
    /// 0 outputs stored apart from presets, 1 with them.
    pub output_config_mode: u8,
    /// 0 master volume stored apart from presets, 1 with them.
    pub master_volume_mode: u8,
}

/// The whole device state, as `dspi_get_state` returns it. Channel-indexed
/// arrays use wire channels; matrices are `[input][output]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DspState {
    /// 0 RP2040, 1 RP2350.
    pub platform_id: u8,
    /// Wire channels.
    pub num_channels: u8,
    /// Input channels.
    pub num_input_channels: u8,
    /// Output channels.
    pub num_output_channels: u8,
    /// Wire format of the bulk image read.
    pub format_version: u8,
    /// Firmware major version.
    pub fw_major: u8,
    /// Firmware minor version.
    pub fw_minor: u8,
    /// Firmware patch version.
    pub fw_patch: u8,
    /// Firmware beta number.
    pub fw_beta: u8,
    /// `COMPAT_*`.
    pub compat: u8,
    /// Global preamp, dB.
    pub preamp_db: f32,
    /// Global bypass.
    pub bypass: bool,
    /// Preamp per input, dB.
    pub input_preamp_db: [f32; MAX_INPUTS],
    /// Master volume, dB.
    pub master_volume_db: f32,
    /// 0 independent of presets, 1 with them.
    pub master_volume_mode: u8,
    /// User volume, dB.
    pub user_volume_db: f32,
    /// User mute.
    pub user_mute: bool,
    /// Loudness on.
    pub loudness_enabled: bool,
    /// Loudness reference level, dB SPL.
    pub loudness_ref_spl: f32,
    /// Loudness intensity, %.
    pub loudness_intensity: f32,
    /// Outputs loudness applies to, one bit each.
    pub loudness_output_mask: u16,
    /// Crossfeed on.
    pub crossfeed_enabled: bool,
    /// Crossfeed preset.
    pub crossfeed_preset: u8,
    /// Crossfeed cutoff, Hz.
    pub crossfeed_freq: f32,
    /// Crossfeed level, dB.
    pub crossfeed_feed: f32,
    /// Crossfeed interaural delay on.
    pub crossfeed_itd: bool,
    /// Output pairs crossfeed applies to, one bit each.
    pub crossfeed_output_pair_mask: u8,
    /// Delay per wire channel, ms.
    pub channel_delays: [f32; MAX_CHANNELS],
    /// Matrix crosspoint on.
    pub matrix_routing: [[bool; MAX_OUTPUTS]; MAX_INPUTS],
    /// Matrix crosspoint gain, dB.
    pub matrix_gain: [[f32; MAX_OUTPUTS]; MAX_INPUTS],
    /// Matrix crosspoint inverted.
    pub matrix_invert: [[bool; MAX_OUTPUTS]; MAX_INPUTS],
    /// Output enabled.
    pub output_enabled: [bool; MAX_OUTPUTS],
    /// Output muted.
    pub output_muted: [bool; MAX_OUTPUTS],
    /// Output gain, dB.
    pub output_gain_db: [f32; MAX_OUTPUTS],
    /// Output delay, ms.
    pub output_delay_ms: [f32; MAX_OUTPUTS],
    /// Limiter on, per output.
    pub limiter_enabled: [bool; MAX_OUTPUTS],
    /// Limiter link group, per output.
    pub limiter_link_group: [u8; MAX_OUTPUTS],
    /// Limiter threshold, dB, per output.
    pub limiter_threshold_db: [f32; MAX_OUTPUTS],
    /// Limiter release, ms, per output.
    pub limiter_release_ms: [f32; MAX_OUTPUTS],
    /// Output pins in use.
    pub num_pin_outputs: u8,
    /// GPIO per output pin.
    pub output_pins: [u8; MAX_PHYSICAL_OUTPUTS],
    /// 0 outputs independent of presets, 1 with them.
    pub output_config_mode: u8,
    /// PEQ bands per wire channel.
    pub filters: [[FilterParams; BANDS_PER_CHANNEL]; MAX_CHANNELS],
    /// Crossover bands per wire channel.
    pub xover: [[FilterParams; MAX_XOVER_BANDS]; MAX_CHANNELS],
    /// Channel names, NUL-terminated UTF-8.
    pub channel_names: [[u8; CHANNEL_NAME_LEN]; MAX_CHANNELS],
    /// Leveller on.
    pub leveller_enabled: bool,
    /// Leveller speed.
    pub leveller_speed: u8,
    /// Leveller look-ahead on.
    pub leveller_lookahead: bool,
    /// Leveller amount, %.
    pub leveller_amount: f32,
    /// Leveller maximum gain, dB.
    pub leveller_max_gain_db: f32,
    /// Leveller gate, dB.
    pub leveller_gate_db: f32,
    /// Channels the leveller listens to.
    pub leveller_detector_mask: u8,
    /// Channels the leveller acts on.
    pub leveller_apply_mask: u8,
    /// Active input source.
    pub input_source: u8,
    /// LG Sound Sync on.
    pub lg_sound_sync_enabled: bool,
    /// S/PDIF inputs enabled, one bit each.
    pub spdif_inputs_enabled: u8,
    /// ADAT input on.
    pub adat_input_enabled: bool,
    /// S/PDIF receiver GPIOs.
    pub spdif_rx_pins: [u8; 4],
    /// I2S receiver GPIOs.
    pub i2s_rx_pins: [u8; 4],
    /// I2S input channels.
    pub i2s_input_channels: u8,
    /// I2S input rate code.
    pub i2s_input_rate: u8,
    /// I2S clock mode, 0 master, 1 slave.
    pub i2s_clock_mode: u8,
    /// ADAT input GPIO.
    pub adat_input_pin: u8,
    /// ADAT input clock mode.
    pub adat_input_clock_mode: u8,
    /// Psychoacoustic bass on.
    pub psybass_enabled: bool,
    /// Outputs it applies to.
    pub psybass_output_mask: u16,
    /// Its cutoff, Hz.
    pub psybass_cutoff_hz: f32,
    /// Its harmonics level, dB.
    pub psybass_harmonics_db: f32,
    /// Its drive, dB.
    pub psybass_drive_db: f32,
    /// Its character, %.
    pub psybass_character_pct: f32,
    /// Its original-signal level, dB.
    pub psybass_original_db: f32,
    /// Upmixer parameters by `UPMIX_PARAM_*`.
    pub upmix: [f32; UPMIX_PARAM_COUNT],
    /// Subharmonic synthesizer parameters by `SUBHARM_PARAM_*`.
    pub subharm: [f32; SUBHARM_PARAM_COUNT],
    /// Subharmonic solo.
    pub subharm_solo: bool,
    /// Tube parameters by `TUBE_PARAM_*`.
    pub tube: [f32; TUBE_PARAM_COUNT],
    /// Output type per output pair.
    pub output_types: [u8; 4],
    /// I2S BCK GPIO.
    pub i2s_bck_pin: u8,
    /// I2S BCK GPIO in slave mode.
    pub i2s_bck_pin_slave: u8,
    /// I2S clock pin mode.
    pub i2s_clock_pin_mode: u8,
    /// Master clock on.
    pub mck_enabled: bool,
    /// Master clock GPIO.
    pub mck_pin: u8,
    /// Master clock multiplier.
    pub mck_multiplier: u8,
    /// ADAT output on.
    pub adat_out_enabled: bool,
    /// ADAT output GPIO.
    pub adat_out_pin: u8,
    /// The firmware has the DAC hardware mute.
    pub dac_mute_supported: bool,
    /// DAC hardware mute on.
    pub dac_mute_enabled: bool,
    /// The mute line is active low.
    pub dac_mute_active_low: bool,
    /// Mute line GPIO.
    pub dac_mute_pin: u8,
    /// Mute hold, ms.
    pub dac_mute_hold_ms: u16,
    /// Mute release, ms.
    pub dac_mute_release_ms: u16,
    /// The firmware has the control interfaces.
    pub ctrl_iface_supported: bool,
    /// UART control interface.
    pub uart: UartConfig,
    /// I2C control interface.
    pub i2c: I2cConfig,
    /// Their status.
    pub ctrl_status: CtrlIfaceStatus,
    /// What core 1 runs.
    pub core1_mode: u8,
    /// One bit per occupied preset slot.
    pub preset_occupied: u16,
    /// Preset names, NUL-terminated UTF-8.
    pub preset_names: [[u8; CHANNEL_NAME_LEN]; MAX_PRESETS],
    /// The active preset slot.
    pub active_preset_slot: u8,
    /// Preset startup mode.
    pub preset_startup_mode: u8,
    /// Preset loaded at boot.
    pub preset_default_slot: u8,
    /// `bulk_raw` holds an image.
    pub bulk_valid: bool,
    /// The bulk parameter image as read.
    pub bulk_raw: [u8; BULK_PARAMS_SIZE],
}

/// What a notification poll found.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NotifyResult {
    /// `NOTIFY_*` bits.
    pub flags: u32,
    /// Channels whose filters changed, one bit each.
    pub filter_channels: u32,
}

/// An analyser request.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RtaRequest {
    /// `RTA_TAP_*`.
    pub tap: u8,
    /// Channels, one bit each.
    pub mask: u16,
    /// FFT bins wanted too.
    pub wants_bins: bool,
    /// Transform size as a power of two.
    pub fft_order: u8,
    /// Averaging time, ms.
    pub avg_ms: u16,
    /// Peak decay, dB/s.
    pub peak_decay_db_s: u8,
}

/// What the analyser can do.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaCapsInfo {
    /// The firmware has the analyser.
    pub supported: bool,
    /// Input channels it can watch.
    pub input_channels: u8,
    /// Output channels it can watch.
    pub output_channels: u8,
    /// Smallest transform order.
    pub order_min: u8,
    /// Largest transform order.
    pub order_max: u8,
    /// Default transform order.
    pub order_default: u8,
    /// Bands from the bass filter bank.
    pub bass_bands: u8,
    /// Bands in all.
    pub max_bands: u8,
    /// Dynamic range, dB.
    pub dynamic_range_db: u8,
    /// Bass bank dynamic range, dB.
    pub bass_dynamic_range_db: u16,
    /// Band centres, Hz.
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

/// One channel's band levels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaChannelFrame {
    /// Channel.
    pub channel: u8,
    /// Bands valid in the arrays.
    pub n_bands: u8,
    /// Nothing above the floor.
    pub silent: bool,
    /// Averaged level per band, dB.
    pub avg_db: [f32; RTA_MAX_BANDS],
    /// Peak level per band, dB.
    pub peak_db: [f32; RTA_MAX_BANDS],
}

impl Default for RtaChannelFrame {
    /// No bands, every level at the floor.
    fn default() -> Self {
        Self {
            channel: 0,
            n_bands: 0,
            silent: false,
            avg_db: [RTA_FLOOR_DB; RTA_MAX_BANDS],
            peak_db: [RTA_FLOOR_DB; RTA_MAX_BANDS],
        }
    }
}

/// The analyser's latest data.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaSnapshot {
    /// Increases with every new frame.
    pub version: u32,
    /// Increases with every status read.
    pub telemetry_version: u32,
    /// `RTA_TAP_*` of the frames.
    pub tap: u8,
    /// Frames valid.
    pub count: u8,
    /// The frames.
    pub frames: [RtaChannelFrame; RTA_MAX_CHANNELS],
    /// Channels that have had a frame, one bit each.
    pub populated: u64,
    /// Bins valid.
    pub bins_count: u16,
    /// Channel of the bins.
    pub bins_channel: u8,
    /// Sample rate of the bins, Hz.
    pub bins_sample_rate: u32,
    /// Bin levels, dB.
    pub bins_db: [f32; RTA_MAX_BINS],
    /// The analyser runs.
    pub running: bool,
    /// Channels analysed.
    pub live_count: u8,
    /// Channels analysed, one bit each.
    pub live_mask: u16,
    /// Frames per second.
    pub frames_per_s: u16,
    /// Processing time, µs per second.
    pub busy_us_per_s: u16,
    /// Time of the last frame, µs.
    pub last_frame_us: u16,
    /// Bass bank time, µs per second.
    pub bass_busy_us_per_s: u16,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// First band shown.
    pub first_band: u8,
    /// Transform order in use.
    pub fft_order: u8,
    /// The device refused the last configuration.
    pub config_rejected: bool,
    /// Seconds between frames.
    pub refresh_interval_s: f32,
}

impl Default for RtaSnapshot {
    /// Nothing analysed: empty frames, bins at the floor.
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
            first_band: 0,
            fft_order: 0,
            config_rejected: false,
            refresh_interval_s: 0.0,
        }
    }
}

/// A signal generator configuration.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenConfig {
    /// Signal type id.
    pub signal_type: u8,
    /// Channels it plays on.
    pub channel_mask: u16,
    /// Channels it plays inverted on.
    pub invert_mask: u16,
    /// `SIGGEN_FLAG_*`.
    pub flags: u8,
    /// Level, dBFS.
    pub level_db: f32,
    /// Duration, ms; 0 continuous.
    pub duration_ms: u32,
    /// Repeats.
    pub repeat: u16,
    /// Gap between repeats, ms.
    pub gap_ms: u16,
    /// The type's parameters.
    pub p: [f32; 4],
}

/// The signal generator's status.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenStatus {
    /// Running state.
    pub state: u8,
    /// Signal type id.
    pub signal_type: u8,
    /// Channel playing now.
    pub active_channel: u8,
    /// Time played, ms.
    pub elapsed_ms: u32,
    /// Cycles completed.
    pub cycles_done: u16,
    /// Why it stopped.
    pub stop_reason: u8,
    /// Frequency now, Hz.
    pub current_freq: f32,
}

/// One parameter of a signal type.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenParam {
    /// What it means.
    pub semantic: u8,
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// Default value.
    pub def: f32,
}

/// A signal type.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenTypeInfo {
    /// Its id.
    pub id: u8,
    /// Its name, NUL-terminated.
    pub name: [u8; 9],
    /// Timing class.
    pub timing: u8,
    /// Its parameters.
    pub params: [SiggenParam; 4],
}

/// What the signal generator can do.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SiggenCaps {
    /// The firmware has it.
    pub supported: bool,
    /// Types valid in `types`.
    pub type_count: u8,
    /// Output channels.
    pub output_channels: u8,
    /// Most tones of the multitone signal.
    pub multitone_max: u8,
    /// Valid channel bits.
    pub valid_mask: u16,
    /// The types.
    pub types: [SiggenTypeInfo; SIGGEN_TYPE_COUNT],
}

/// One S/PDIF output's buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpdifBufferStats {
    /// Free buffers.
    pub free: u8,
    /// Buffers prepared.
    pub prepared: u8,
    /// Buffers playing.
    pub playing: u8,
    /// Fill now, %.
    pub fill_pct: u8,
    /// Lowest fill, %.
    pub min_pct: u8,
    /// Highest fill, %.
    pub max_pct: u8,
}

/// The PDM output's buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PdmBufferStats {
    /// DMA fill now, %.
    pub dma_fill_pct: u8,
    /// Lowest DMA fill, %.
    pub dma_min_pct: u8,
    /// Highest DMA fill, %.
    pub dma_max_pct: u8,
    /// Ring fill now, %.
    pub ring_fill_pct: u8,
    /// Lowest ring fill, %.
    pub ring_min_pct: u8,
    /// Highest ring fill, %.
    pub ring_max_pct: u8,
}

/// Output buffer statistics.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BufferStats {
    /// S/PDIF outputs reported.
    pub num_spdif: u8,
    /// The PDM output runs.
    pub pdm_active: bool,
    /// USB audio streams.
    pub streaming: bool,
    /// Increases with every reading.
    pub sequence: u16,
    /// Per S/PDIF output.
    pub spdif: [SpdifBufferStats; 4],
    /// The PDM output.
    pub pdm: PdmBufferStats,
}

/// The S/PDIF receiver's status.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpdifRxStatus {
    /// Receiver state.
    pub state: u8,
    /// Input source.
    pub input_source: u8,
    /// Locks so far (wraps).
    pub lock_count: u8,
    /// Losses of lock so far (wraps).
    pub loss_count: u8,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Parity errors.
    pub parity_errors: u32,
    /// FIFO fill, %.
    pub fifo_fill_pct: u16,
    /// The receiver library's state.
    pub lib_state: u8,
    /// Stable callbacks.
    pub stable_callbacks: u8,
    /// Lost callbacks.
    pub lost_callbacks: u8,
}

/// Who chose the analyser configuration the device holds.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RtaOwnership {
    /// Another console's configuration; the fields below are 0 otherwise.
    pub following: bool,
    /// The session that pushed it, 0 when unknown.
    pub origin: u16,
    /// Its tap.
    pub tap: u8,
    /// Its channels.
    pub mask: u16,
    /// Its transform order.
    pub fft_order: u8,
}

// ── From the core's records ───────────────────────────────────────────

impl From<&r::FilterParams> for FilterParams {
    fn from(f: &r::FilterParams) -> Self {
        Self {
            filter_type: f.filter_type,
            bypass: f.bypass,
            freq: f.freq,
            q: f.q,
            gain: f.gain,
            qp: f.qp,
        }
    }
}

impl From<&r::InputLockStatus> for InputLockStatus {
    fn from(s: &r::InputLockStatus) -> Self {
        Self {
            state: s.state,
            clock_mode: s.clock_mode,
            detected_rate: s.detected_rate,
            measured_hz: s.measured_hz,
            rate_ok: s.rate_ok,
            lock_count: s.lock_count,
            loss_count: s.loss_count,
        }
    }
}

impl From<&r::AdatOutStatus> for AdatOutStatus {
    fn from(s: &r::AdatOutStatus) -> Self {
        Self {
            enabled: s.enabled,
            active: s.active,
            pin: s.pin,
            rate_ok: s.rate_ok,
            resync_count: s.resync_count,
            slip_count: s.slip_count,
        }
    }
}

impl From<&r::LgStatus> for LgStatus {
    fn from(s: &r::LgStatus) -> Self {
        Self {
            enabled: s.enabled,
            present: s.present,
            volume: s.volume,
            muted: s.muted,
        }
    }
}

impl From<&r::UartConfig> for UartConfig {
    fn from(c: &r::UartConfig) -> Self {
        Self {
            enabled: c.enabled,
            tx_pin: c.tx_pin,
            rx_pin: c.rx_pin,
            notify: c.notify,
            baud: c.baud,
        }
    }
}

impl From<&r::I2cConfig> for I2cConfig {
    fn from(c: &r::I2cConfig) -> Self {
        Self {
            enabled: c.enabled,
            sda_pin: c.sda_pin,
            scl_pin: c.scl_pin,
            address: c.address,
        }
    }
}

impl From<&r::CtrlIfaceStatus> for CtrlIfaceStatus {
    fn from(s: &r::CtrlIfaceStatus) -> Self {
        Self {
            uart_last_status: s.uart_last_status,
            uart_live: s.uart_live,
            i2c_last_status: s.i2c_last_status,
            i2c_live: s.i2c_live,
            protocol_version: s.protocol_version,
        }
    }
}

impl From<&r::UpmixStatus> for UpmixStatus {
    fn from(s: &r::UpmixStatus) -> Self {
        Self {
            active: s.active,
            parked_reason: s.parked_reason,
            correlation: s.correlation,
            balance: s.balance,
            center_gain: s.center_gain,
            ls_gain: s.ls_gain,
            rs_gain: s.rs_gain,
        }
    }
}

impl From<&r::SystemStatus> for SystemStatus {
    /// `stale` and `age_ms` have no place in the C record.
    fn from(s: &r::SystemStatus) -> Self {
        Self {
            peaks: copied(&s.peaks, 0.0),
            cpu0: s.cpu0,
            cpu1: s.cpu1,
            clip_flags: s.clip_flags,
            active_input_channels: s.active_input_channels,
            num_channels: s.num_channels,
            // -- WP P0M --
            peak_hold: copied(&s.peak_hold, 0.0),
            peak_max: copied(&s.peak_max, 0.0),
            clip_hold: s.clip_hold,
        }
    }
}

impl From<&r::DeviceInfo> for DeviceInfo {
    /// The serial is cut to 63 bytes (at a character boundary) and terminated.
    fn from(d: &r::DeviceInfo) -> Self {
        let serial: [u8; 64] = name_buf(&d.serial);
        let len = serial.iter().position(|&b| b == 0).unwrap_or(serial.len());
        Self {
            serial,
            serial_len: u32::try_from(len).unwrap_or(0),
            location_id: d.location_id,
        }
    }
}

impl From<&r::PresetDirectory> for PresetDirectory {
    fn from(d: &r::PresetDirectory) -> Self {
        Self {
            occupied_mask: d.occupied_mask,
            startup_mode: d.startup_mode,
            default_slot: d.default_slot,
            last_active: d.last_active,
            output_config_mode: d.output_config_mode,
            master_volume_mode: d.master_volume_mode,
        }
    }
}

impl From<&r::DspState> for DspState {
    /// Field for field; the record's `crossfeed_pair_map` and `*_present`
    /// flags have no place in the C layout.
    fn from(s: &r::DspState) -> Self {
        let band = |f: &r::FilterParams| FilterParams::from(f);
        Self {
            platform_id: s.platform_id,
            num_channels: s.num_channels,
            num_input_channels: s.num_input_channels,
            num_output_channels: s.num_output_channels,
            format_version: s.format_version,
            fw_major: s.fw_major,
            fw_minor: s.fw_minor,
            fw_patch: s.fw_patch,
            fw_beta: s.fw_beta,
            compat: s.compat,
            preamp_db: s.preamp_db,
            bypass: s.bypass,
            input_preamp_db: copied(&s.input_preamp_db, 0.0),
            master_volume_db: s.master_volume_db,
            master_volume_mode: s.master_volume_mode,
            user_volume_db: s.user_volume_db,
            user_mute: s.user_mute,
            loudness_enabled: s.loudness_enabled,
            loudness_ref_spl: s.loudness_ref_spl,
            loudness_intensity: s.loudness_intensity,
            loudness_output_mask: s.loudness_output_mask,
            crossfeed_enabled: s.crossfeed_enabled,
            crossfeed_preset: s.crossfeed_preset,
            crossfeed_freq: s.crossfeed_freq,
            crossfeed_feed: s.crossfeed_feed,
            crossfeed_itd: s.crossfeed_itd,
            crossfeed_output_pair_mask: s.crossfeed_output_pair_mask,
            channel_delays: copied(&s.channel_delays, 0.0),
            matrix_routing: table(&s.matrix_routing, false),
            matrix_gain: table(&s.matrix_gain, 0.0),
            matrix_invert: table(&s.matrix_invert, false),
            output_enabled: copied(&s.output_enabled, false),
            output_muted: copied(&s.output_muted, false),
            output_gain_db: copied(&s.output_gain_db, 0.0),
            output_delay_ms: copied(&s.output_delay_ms, 0.0),
            limiter_enabled: copied(&s.limiter_enabled, false),
            limiter_link_group: copied(&s.limiter_link_group, 0),
            limiter_threshold_db: copied(&s.limiter_threshold_db, 0.0),
            limiter_release_ms: copied(&s.limiter_release_ms, 0.0),
            num_pin_outputs: s.num_pin_outputs,
            output_pins: copied(&s.output_pins, 0),
            output_config_mode: s.output_config_mode,
            filters: array_from(
                &s.filters,
                [FilterParams::default(); BANDS_PER_CHANNEL],
                |ch| array_from(ch, FilterParams::default(), band),
            ),
            xover: array_from(&s.xover, [FilterParams::default(); MAX_XOVER_BANDS], |ch| {
                array_from(ch, FilterParams::default(), band)
            }),
            channel_names: array_from(&s.channel_names, [0; CHANNEL_NAME_LEN], |n| name_buf(n)),
            leveller_enabled: s.leveller_enabled,
            leveller_speed: s.leveller_speed,
            leveller_lookahead: s.leveller_lookahead,
            leveller_amount: s.leveller_amount,
            leveller_max_gain_db: s.leveller_max_gain_db,
            leveller_gate_db: s.leveller_gate_db,
            leveller_detector_mask: s.leveller_detector_mask,
            leveller_apply_mask: s.leveller_apply_mask,
            input_source: s.input_source,
            lg_sound_sync_enabled: s.lg_sound_sync_enabled,
            spdif_inputs_enabled: s.spdif_inputs_enabled,
            adat_input_enabled: s.adat_input_enabled,
            spdif_rx_pins: copied(&s.spdif_rx_pins, 0),
            i2s_rx_pins: copied(&s.i2s_rx_pins, 0),
            i2s_input_channels: s.i2s_input_channels,
            i2s_input_rate: s.i2s_input_rate,
            i2s_clock_mode: s.i2s_clock_mode,
            adat_input_pin: s.adat_input_pin,
            adat_input_clock_mode: s.adat_input_clock_mode,
            psybass_enabled: s.psybass_enabled,
            psybass_output_mask: s.psybass_output_mask,
            psybass_cutoff_hz: s.psybass_cutoff_hz,
            psybass_harmonics_db: s.psybass_harmonics_db,
            psybass_drive_db: s.psybass_drive_db,
            psybass_character_pct: s.psybass_character_pct,
            psybass_original_db: s.psybass_original_db,
            upmix: copied(&s.upmix, 0.0),
            subharm: copied(&s.subharm, 0.0),
            subharm_solo: s.subharm_solo,
            tube: copied(&s.tube, 0.0),
            output_types: copied(&s.output_types, 0),
            i2s_bck_pin: s.i2s_bck_pin,
            i2s_bck_pin_slave: s.i2s_bck_pin_slave,
            i2s_clock_pin_mode: s.i2s_clock_pin_mode,
            mck_enabled: s.mck_enabled,
            mck_pin: s.mck_pin,
            mck_multiplier: s.mck_multiplier,
            adat_out_enabled: s.adat_out_enabled,
            adat_out_pin: s.adat_out_pin,
            dac_mute_supported: s.dac_mute_supported,
            dac_mute_enabled: s.dac_mute_enabled,
            dac_mute_active_low: s.dac_mute_active_low,
            dac_mute_pin: s.dac_mute_pin,
            dac_mute_hold_ms: s.dac_mute_hold_ms,
            dac_mute_release_ms: s.dac_mute_release_ms,
            ctrl_iface_supported: s.ctrl_iface_supported,
            uart: UartConfig::from(&s.uart),
            i2c: I2cConfig::from(&s.i2c),
            ctrl_status: CtrlIfaceStatus::from(&s.ctrl_status),
            core1_mode: s.core1_mode,
            preset_occupied: s.preset_occupied,
            preset_names: array_from(&s.preset_names, [0; CHANNEL_NAME_LEN], |n| name_buf(n)),
            active_preset_slot: s.active_preset_slot,
            preset_startup_mode: s.preset_startup_mode,
            preset_default_slot: s.preset_default_slot,
            bulk_valid: s.bulk_valid,
            bulk_raw: copied(&s.bulk_raw, 0),
        }
    }
}

impl From<&r::NotifyResult> for NotifyResult {
    fn from(n: &r::NotifyResult) -> Self {
        Self {
            flags: n.flags,
            filter_channels: n.filter_channels,
        }
    }
}

impl From<&r::RtaOwnership> for RtaOwnership {
    fn from(o: &r::RtaOwnership) -> Self {
        Self {
            following: o.following,
            origin: o.origin,
            tap: o.tap,
            mask: o.mask,
            fft_order: o.fft_order,
        }
    }
}

impl From<&r::RtaRequest> for RtaRequest {
    /// `watching` stays behind: the C caller always asks to drive.
    fn from(q: &r::RtaRequest) -> Self {
        Self {
            tap: q.tap,
            mask: q.mask,
            wants_bins: q.wants_bins,
            fft_order: q.fft_order,
            avg_ms: q.avg_ms,
            peak_decay_db_s: q.peak_decay_db_s,
        }
    }
}

impl From<&r::RtaCapsInfo> for RtaCapsInfo {
    fn from(c: &r::RtaCapsInfo) -> Self {
        Self {
            supported: c.supported,
            input_channels: c.input_channels,
            output_channels: c.output_channels,
            order_min: c.order_min,
            order_max: c.order_max,
            order_default: c.order_default,
            bass_bands: c.bass_bands,
            max_bands: c.max_bands,
            dynamic_range_db: c.dynamic_range_db,
            bass_dynamic_range_db: c.bass_dynamic_range_db,
            centres: copied(&c.centres, 0),
        }
    }
}

impl From<&r::RtaChannelFrame> for RtaChannelFrame {
    /// Bands past the record's are at the floor.
    fn from(f: &r::RtaChannelFrame) -> Self {
        Self {
            channel: f.channel,
            n_bands: f.n_bands,
            silent: f.silent,
            avg_db: copied(&f.avg_db, RTA_FLOOR_DB),
            peak_db: copied(&f.peak_db, RTA_FLOOR_DB),
        }
    }
}

impl From<&r::RtaSnapshot> for RtaSnapshot {
    /// Missing frames are empty ones, missing bins at the floor.
    fn from(s: &r::RtaSnapshot) -> Self {
        Self {
            version: s.version,
            telemetry_version: s.telemetry_version,
            tap: s.tap,
            count: s.count,
            frames: array_from(&s.frames, RtaChannelFrame::default(), |f| {
                RtaChannelFrame::from(f)
            }),
            populated: s.populated,
            bins_count: s.bins_count,
            bins_channel: s.bins_channel,
            bins_sample_rate: s.bins_sample_rate,
            bins_db: copied(&s.bins_db, RTA_FLOOR_DB),
            running: s.running,
            live_count: s.live_count,
            live_mask: s.live_mask,
            frames_per_s: s.frames_per_s,
            busy_us_per_s: s.busy_us_per_s,
            last_frame_us: s.last_frame_us,
            bass_busy_us_per_s: s.bass_busy_us_per_s,
            sample_rate: s.sample_rate,
            first_band: s.first_band,
            fft_order: s.fft_order,
            config_rejected: s.config_rejected,
            refresh_interval_s: s.refresh_interval_s,
        }
    }
}

impl From<&r::SiggenConfig> for SiggenConfig {
    fn from(c: &r::SiggenConfig) -> Self {
        Self {
            signal_type: c.signal_type,
            channel_mask: c.channel_mask,
            invert_mask: c.invert_mask,
            flags: c.flags,
            level_db: c.level_db,
            duration_ms: c.duration_ms,
            repeat: c.repeat,
            gap_ms: c.gap_ms,
            p: copied(&c.p, 0.0),
        }
    }
}

impl From<&r::SiggenStatus> for SiggenStatus {
    fn from(s: &r::SiggenStatus) -> Self {
        Self {
            state: s.state,
            signal_type: s.signal_type,
            active_channel: s.active_channel,
            elapsed_ms: s.elapsed_ms,
            cycles_done: s.cycles_done,
            stop_reason: s.stop_reason,
            current_freq: s.current_freq,
        }
    }
}

impl From<&r::SiggenParam> for SiggenParam {
    fn from(p: &r::SiggenParam) -> Self {
        Self {
            semantic: p.semantic,
            min: p.min,
            max: p.max,
            def: p.def,
        }
    }
}

impl From<&r::SiggenTypeInfo> for SiggenTypeInfo {
    /// The name is cut to 8 bytes and terminated.
    fn from(t: &r::SiggenTypeInfo) -> Self {
        Self {
            id: t.id,
            name: name_buf(&t.name),
            timing: t.timing,
            params: array_from(&t.params, SiggenParam::default(), |p| SiggenParam::from(p)),
        }
    }
}

impl From<&r::SiggenCaps> for SiggenCaps {
    fn from(c: &r::SiggenCaps) -> Self {
        Self {
            supported: c.supported,
            type_count: c.type_count,
            output_channels: c.output_channels,
            multitone_max: c.multitone_max,
            valid_mask: c.valid_mask,
            types: array_from(&c.types, SiggenTypeInfo::default(), |t| {
                SiggenTypeInfo::from(t)
            }),
        }
    }
}

impl From<&r::SpdifBufferStats> for SpdifBufferStats {
    fn from(s: &r::SpdifBufferStats) -> Self {
        Self {
            free: s.free,
            prepared: s.prepared,
            playing: s.playing,
            fill_pct: s.fill_pct,
            min_pct: s.min_pct,
            max_pct: s.max_pct,
        }
    }
}

impl From<&r::PdmBufferStats> for PdmBufferStats {
    fn from(s: &r::PdmBufferStats) -> Self {
        Self {
            dma_fill_pct: s.dma_fill_pct,
            dma_min_pct: s.dma_min_pct,
            dma_max_pct: s.dma_max_pct,
            ring_fill_pct: s.ring_fill_pct,
            ring_min_pct: s.ring_min_pct,
            ring_max_pct: s.ring_max_pct,
        }
    }
}

impl From<&r::BufferStats> for BufferStats {
    fn from(s: &r::BufferStats) -> Self {
        Self {
            num_spdif: s.num_spdif,
            pdm_active: s.pdm_active,
            streaming: s.streaming,
            sequence: s.sequence,
            spdif: array_from(&s.spdif, SpdifBufferStats::default(), |b| {
                SpdifBufferStats::from(b)
            }),
            pdm: PdmBufferStats::from(&s.pdm),
        }
    }
}

impl From<&r::SpdifRxStatus> for SpdifRxStatus {
    fn from(s: &r::SpdifRxStatus) -> Self {
        Self {
            state: s.state,
            input_source: s.input_source,
            lock_count: s.lock_count,
            loss_count: s.loss_count,
            sample_rate: s.sample_rate,
            parity_errors: s.parity_errors,
            fifo_fill_pct: s.fifo_fill_pct,
            lib_state: s.lib_state,
            stable_callbacks: s.stable_callbacks,
            lost_callbacks: s.lost_callbacks,
        }
    }
}

// ── Back to the core's records (what a C caller passes in) ────────────

impl From<&FilterParams> for r::FilterParams {
    fn from(f: &FilterParams) -> Self {
        Self {
            filter_type: f.filter_type,
            bypass: f.bypass,
            freq: f.freq,
            q: f.q,
            gain: f.gain,
            qp: f.qp,
        }
    }
}

impl From<&UartConfig> for r::UartConfig {
    fn from(c: &UartConfig) -> Self {
        Self {
            enabled: c.enabled,
            tx_pin: c.tx_pin,
            rx_pin: c.rx_pin,
            notify: c.notify,
            baud: c.baud,
        }
    }
}

impl From<&I2cConfig> for r::I2cConfig {
    fn from(c: &I2cConfig) -> Self {
        Self {
            enabled: c.enabled,
            sda_pin: c.sda_pin,
            scl_pin: c.scl_pin,
            address: c.address,
        }
    }
}

impl From<&RtaRequest> for r::RtaRequest {
    /// A C caller drives the analyser: `watching` is false.
    fn from(q: &RtaRequest) -> Self {
        Self {
            tap: q.tap,
            mask: q.mask,
            wants_bins: q.wants_bins,
            fft_order: q.fft_order,
            avg_ms: q.avg_ms,
            peak_decay_db_s: q.peak_decay_db_s,
            watching: false,
        }
    }
}

impl From<&SiggenConfig> for r::SiggenConfig {
    fn from(c: &SiggenConfig) -> Self {
        Self {
            signal_type: c.signal_type,
            channel_mask: c.channel_mask,
            invert_mask: c.invert_mask,
            flags: c.flags,
            level_db: c.level_db,
            duration_ms: c.duration_ms,
            repeat: c.repeat,
            gap_ms: c.gap_ms,
            p: c.p.to_vec(),
        }
    }
}

impl From<&DeviceInfo> for r::DeviceInfo {
    /// `serial_len` is trusted no further than the buffer and the first NUL.
    fn from(d: &DeviceInfo) -> Self {
        let len = usize::try_from(d.serial_len).unwrap_or(usize::MAX);
        Self {
            serial: text_of(&d.serial, len),
            location_id: d.location_id,
        }
    }
}

// -- WP P0M --
/// The meter ballistics' timing (`dspi_meter_ballistics`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterBallistics {
    /// How long a new peak holds, ms (default 1000).
    pub hold_ms: u32,
    /// How fast a held peak falls after its hold, full scale per second
    /// (default 0.5).
    pub decay_per_s: f32,
    /// How long the clip hold lasts after the last new clip, ms; 0 until
    /// cleared (default 10000).
    pub clip_hold_ms: u32,
}

impl From<r::MeterBallistics> for MeterBallistics {
    fn from(m: r::MeterBallistics) -> Self {
        Self {
            hold_ms: m.hold_ms,
            decay_per_s: m.decay_per_s,
            clip_hold_ms: m.clip_hold_ms,
        }
    }
}
