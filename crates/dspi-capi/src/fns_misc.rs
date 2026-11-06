//! C functions: hardware, hotplug, notify, monitor, RTA, siggen/stats, files,
//! CS, and the additions to the old ABI: extension channel, names,
//! diagnostics, firmware and the demo device (WP Q2b).
//!
//! Failure values: `bool` false; a status byte is the device's `PIN_CONFIG_*`
//! or `PRESET_*` code (0 = success) and 0xFF when it did not answer; `i8` -1;
//! a `u32` text length 0. A failed fetch leaves `out` untouched. A `void`
//! function that fills a record writes its zero value when the handle is null.
//! A misaligned record pointer counts as a null one.
//!
//! **Text functions** copy NUL-terminated UTF-8 into `buf` and return the bytes
//! needed including the NUL. When that is more than `len`, nothing is written:
//! call again with a larger buffer. **Except** `dspi_config_import`,
//! `dspi_filters_import` and `dspi_cs_apply`, which change the device: don't
//! retry those. A reply too long means the write has happened, and a second
//! call applies it again. Give them a large enough buffer the first time.
//!
//! **The added functions** that return a record reply in JSON, built from
//! dspi-core's `Serialize` records: `{"ok":true, <the record's fields in
//! camelCase>}` (a list as `"value"`), or `{"ok":false,"code":<kind>,
//! "error":<sentence>}`. `code` is one of `disconnected`, `transfer`,
//! `refused`, `invalid`, `unsupported`, `read_only`, `locked`, `denied`,
//! `link` or `io`. `unsupported` is how stock firmware answers the
//! extension channel; it is not a fault.

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;

use dspi_core::{
    BootloaderDrive, DeviceEvents, DspiCore, DspiError, NotifyListener, Result, RtaListener,
};
use serde::Serialize;

use crate::types::{
    AdatOutStatus, BufferStats, DeviceEventCallback, I2cConfig, InputLockStatus, LgStatus,
    NotifyCallback, NotifyResult, RtaCapsInfo, RtaRequest, RtaSnapshot, SiggenCaps, SiggenConfig,
    SiggenStatus, SpdifRxStatus, UartConfig,
};
use crate::{FfiCore, guard, with_core};

/// Bytes of IEC 60958 channel status `dspi_fetch_spdif_rx_channel_status` fills.
const CHANNEL_STATUS_LEN: usize = 24;
/// Bytes of a crossfeed pair map.
const PAIR_MAP_LEN: usize = 4;

// ── Helpers ───────────────────────────────────────────────────────────

/// Borrow a NUL-terminated UTF-8 C string: `None` for null or invalid UTF-8.
fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null string argument is NUL-terminated and outlives the
    // call; that is the C caller's contract.
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// `c_str`, with "" for a null or invalid string: the old ABI's way for the
/// text documents, whose parsers then report the empty text.
fn c_text(p: *const c_char) -> String {
    c_str(p).unwrap_or("").to_owned()
}

/// The caller's bytes as a vector; empty for a null pointer.
fn c_bytes(p: *const u8, len: u32) -> Vec<u8> {
    if p.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: a non-null buffer holds `len` bytes; the C caller's contract.
    unsafe { std::slice::from_raw_parts(p, len as usize) }.to_vec()
}

/// Copy `s` into a caller buffer, NUL-terminated, cut at a character boundary
/// if it does not fit. False for a null or empty buffer.
fn copy_out(s: &str, buf: *mut c_char, len: u32) -> bool {
    let Some(room) = (len as usize).checked_sub(1) else {
        return false;
    };
    if buf.is_null() {
        return false;
    }
    let cut = s.len().min(room);
    let n = (0..=cut)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    // SAFETY: `buf` holds `len` bytes (the C caller's contract), n <= len - 1,
    // and a Rust string cannot overlap a buffer the caller owns.
    unsafe {
        ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), n);
        buf.add(n).write(0);
    }
    true
}

/// Copy `s` into `buf` if it fits whole; returns the bytes needed including
/// the NUL, so a caller with too small a buffer can retry.
fn copy_text(s: &str, buf: *mut c_char, len: u32) -> u32 {
    let needed = u32::try_from(s.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .unwrap_or(u32::MAX);
    if needed <= len {
        copy_out(s, buf, len);
    }
    needed
}

/// Copy `src` to `out`, which holds exactly `want` bytes; false, with nothing
/// written, for a null `out` or a reply of another length.
fn copy_bytes(src: &[u8], out: *mut u8, want: usize) -> bool {
    if out.is_null() || src.len() != want {
        return false;
    }
    // SAFETY: `out` holds `want` bytes, the C caller's contract, and src has
    // that length.
    unsafe { ptr::copy_nonoverlapping(src.as_ptr(), out, want) };
    true
}

/// Run a text function on the handle: 0 for a null handle or a panic.
fn text(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
    f: impl FnOnce(&DspiCore) -> String,
) -> u32 {
    with_core(core, 0, |h| copy_text(&f(&h.core), buf, len))
}

/// `Ok` as true; an error, logged, as false.
fn flag(core: *mut FfiCore, f: impl FnOnce(&DspiCore) -> Result<()>) -> bool {
    with_core(core, false, |h| match f(&h.core) {
        Ok(()) => true,
        Err(e) => {
            log::debug!("dspi-capi: {e}");
            false
        }
    })
}

/// `Ok` as 0, a device's refusal as its status byte, anything else as 0xFF.
fn status(core: *mut FfiCore, f: impl FnOnce(&DspiCore) -> Result<()>) -> u8 {
    with_core(core, 0xFF, |h| match f(&h.core) {
        Ok(()) => 0,
        Err(DspiError::Refused { status }) => status,
        Err(e) => {
            log::debug!("dspi-capi: {e}");
            0xFF
        }
    })
}

/// Fetch a record into `out`, converted to its C layout.
fn fetch<R, C>(core: *mut FfiCore, out: *mut C, f: impl FnOnce(&DspiCore) -> Result<R>) -> bool
where
    C: for<'a> From<&'a R>,
{
    if out.is_null() || !out.is_aligned() {
        return false;
    }
    with_core(core, false, |h| match f(&h.core) {
        Ok(v) => {
            // SAFETY: not null, aligned, and points to a `C`; the C caller's
            // contract.
            unsafe { out.write(C::from(&v)) };
            true
        }
        Err(e) => {
            log::debug!("dspi-capi: {e}");
            false
        }
    })
}

/// Fetch a plain value into `out`.
fn fetch_value<T>(core: *mut FfiCore, out: *mut T, f: impl FnOnce(&DspiCore) -> Result<T>) -> bool {
    if out.is_null() || !out.is_aligned() {
        return false;
    }
    with_core(core, false, |h| match f(&h.core) {
        Ok(v) => {
            // SAFETY: not null, aligned, and points to a `T`; the C caller's
            // contract.
            unsafe { out.write(v) };
            true
        }
        Err(e) => {
            log::debug!("dspi-capi: {e}");
            false
        }
    })
}

/// The `code` of a JSON failure reply (module docs).
fn error_code(e: &DspiError) -> &'static str {
    match e {
        DspiError::Disconnected => "disconnected",
        DspiError::Transfer { .. } => "transfer",
        DspiError::Refused { .. } => "refused",
        DspiError::InvalidArgument { .. } => "invalid",
        DspiError::Unsupported => "unsupported",
        DspiError::ReadOnly => "read_only",
        DspiError::Locked => "locked",
        DspiError::Denied => "denied",
        DspiError::Link { .. } => "link",
        DspiError::Io { .. } => "io",
    }
}

/// A result as the added functions' JSON reply (module docs).
///
/// `ok` is inserted among the record's own fields, so no record that comes
/// through here may have a field named `ok`: it would be overwritten. A test
/// checks every such record (`wp_q2b_no_reply_record_has_a_field_named_ok`).
fn json_reply<T: Serialize>(r: Result<T>) -> String {
    let fields = r.and_then(|v| {
        serde_json::to_value(v).map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })
    });
    let reply = match fields {
        Ok(serde_json::Value::Object(mut m)) => {
            m.insert("ok".to_owned(), serde_json::Value::Bool(true));
            serde_json::Value::Object(m)
        }
        Ok(v) => serde_json::json!({ "ok": true, "value": v }),
        Err(e) => serde_json::json!({
            "ok": false,
            "code": error_code(&e),
            "error": e.to_string(),
        }),
    };
    reply.to_string()
}

// ── Listeners over C function pointers ────────────────────────────────

/// A C wake-up callback and its `user_data`, as the core's listeners.
struct CWake {
    callback: NotifyCallback,
    user_data: *mut c_void,
}

// SAFETY: the callback runs on core threads by the ABI's documented contract
// (the caller's `user_data` must allow that); nothing here touches the pointer.
unsafe impl Send for CWake {}
// SAFETY: as for Send: the pointer is only passed back to the caller.
unsafe impl Sync for CWake {}

impl NotifyListener for CWake {
    fn on_notifications(&self) {
        (self.callback)(self.user_data);
    }
}

impl RtaListener for CWake {
    fn on_frame(&self) {
        (self.callback)(self.user_data);
    }
}

/// The C hotplug callback and its `user_data`, as the core's listener.
struct CHotplug {
    callback: DeviceEventCallback,
    user_data: *mut c_void,
}

// SAFETY: as for `CWake`.
unsafe impl Send for CHotplug {}
// SAFETY: as for `CWake`.
unsafe impl Sync for CHotplug {}

impl CHotplug {
    fn fire(&self, event: u8, serial: &str) {
        // A serial with an inner NUL reads as cut there, never as a failure.
        let mut z = Vec::with_capacity(serial.len().saturating_add(1));
        z.extend_from_slice(serial.as_bytes());
        z.push(0);
        (self.callback)(event, z.as_ptr().cast::<c_char>(), self.user_data);
    }
}

impl DeviceEvents for CHotplug {
    fn on_arrived(&self, serial: String) {
        self.fire(0, &serial);
    }

    fn on_departed(&self, serial: String) {
        self.fire(1, &serial);
    }
}

// ── Pins and output configuration ─────────────────────────────────────

/// Returns a PIN_CONFIG_* status (0 = success), 0xFF on communication error.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_pin(core: *mut FfiCore, slot: u8, pin: u8) -> u8 {
    status(core, |c| c.set_output_pin(slot, pin))
}

/// Returns the pin number, 0xFF on error.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_pin(core: *mut FfiCore, slot: u8) -> u8 {
    with_core(core, 0xFF, |h| {
        h.core.fetch_output_pin(slot).unwrap_or(0xFF)
    })
}

/// 0 = output config stored independently, 1 = saved with presets.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_config_mode(core: *mut FfiCore, mode: u8) -> bool {
    flag(core, |c| c.set_output_config_mode(mode))
}

/// Persist the live output config (independent mode). PRESET_* code, 0xFF on error.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_output_config(core: *mut FfiCore) -> u8 {
    status(core, |c| c.save_output_config())
}

// ── System ────────────────────────────────────────────────────────────

/// Restart the device into its USB bootloader.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_enter_bootloader(core: *mut FfiCore) -> bool {
    flag(core, |c| c.enter_bootloader())
}

/// Core 1 mode (0 = idle, 1 = PDM, 2 = EQ worker), -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_core1_mode(core: *mut FfiCore) -> i8 {
    with_core(core, -1, |h| match h.core.fetch_core1_mode() {
        Ok(mode) => i8::try_from(mode).unwrap_or(-1),
        Err(_) => -1,
    })
}

/// 1 if enabling `output` would conflict with Core 1's mode, 0 if not, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_check_core1_conflict(core: *mut FfiCore, output: u8) -> i8 {
    with_core(core, -1, |h| match h.core.check_core1_conflict(output) {
        Ok(conflict) => i8::from(conflict),
        Err(_) => -1,
    })
}

// ── Hot-plug ──────────────────────────────────────────────────────────

/// Register a callback for device arrival/departure events; NULL clears it,
/// waiting for a call in progress on another thread. False for a null handle.
/// It runs on the thread that calls `dspi_poll_hotplug`, and must not set
/// listeners from inside the call.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_hotplug_callback(
    core: *mut FfiCore,
    // Spelled out: cbindgen renders `Option<DeviceEventCallback>` as a struct.
    callback: Option<extern "C" fn(event: u8, serial: *const c_char, user_data: *mut c_void)>,
    user_data: *mut c_void,
) -> bool {
    with_core(core, false, |h| {
        let listener = callback.map(|callback| {
            Arc::new(CHotplug {
                callback,
                user_data,
            }) as Arc<dyn DeviceEvents>
        });
        h.core.set_hotplug_callback(listener);
        true
    })
}

/// Poll for hot-plug changes (~every 500 ms). Fires the registered callback
/// for arrivals (event 0) and departures (event 1), reconnects the selected
/// device when it comes back and closes one that went away. A reconnect runs
/// here, so the call can take a few hundred milliseconds on a replug. Call
/// `dspi_scan_devices` first, or every unit present is announced as arrived.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_poll_hotplug(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.poll_hotplug());
}

// ── Hardware I/O (outputs, clocks, inputs). Status-returning calls give
//    the PIN_CONFIG_* code, or 255 when the device did not answer. ──────

/// Slot type: 0 = S/PDIF, 1 = I2S.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_type(core: *mut FfiCore, slot: u8, kind: u8) -> u8 {
    status(core, |c| c.set_output_type(slot, kind))
}

/// I2S BCK pin (LRCLK = +1); role 0 = master/shared, 1 = slave pair.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_bck_pin(core: *mut FfiCore, role: u8, pin: u8) -> u8 {
    status(core, |c| c.set_i2s_bck_pin(role, pin))
}

/// 0 = shared clock pins, 1 = separate slave pins.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_clock_pin_mode(core: *mut FfiCore, mode: u8) -> u8 {
    status(core, |c| c.set_i2s_clock_pin_mode(mode))
}

/// Master clock output on/off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_mck_enabled(enabled))
}

/// Master clock pin (MCK off first).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_mck_pin(pin))
}

/// 0 = 128 x fs, 1 = 256 x fs.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_multiplier(core: *mut FfiCore, mult: u8) -> u8 {
    status(core, |c| c.set_mck_multiplier(mult))
}

/// ADAT optical output (RP2350).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_out_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_adat_out_enabled(enabled))
}

/// ADAT output data pin.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_out_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_adat_out_pin(pin))
}

/// S/PDIF input RX pin (index 0-3).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_spdif_rx_pin(core: *mut FfiCore, index: u8, pin: u8) -> u8 {
    status(core, |c| c.set_spdif_rx_pin(index, pin))
}

/// Enable S/PDIF input 2-4 (index 1-3).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_spdif_input_enabled(core: *mut FfiCore, index: u8, enabled: bool) -> u8 {
    status(core, |c| c.set_spdif_input_enabled(index, enabled))
}

/// I2S input data pin of a pair (0-3).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_rx_pin(core: *mut FfiCore, pair: u8, pin: u8) -> u8 {
    status(core, |c| c.set_i2s_rx_pin(pair, pin))
}

/// I2S input channels: 2, 4, 6 or 8.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_input_channels(core: *mut FfiCore, channels: u8) -> u8 {
    status(core, |c| c.set_i2s_input_channels(channels))
}

/// ADAT input on/off (needs a pin).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_adat_input_enabled(enabled))
}

/// ADAT input data pin; 0xFF clears it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_adat_input_pin(pin))
}

/// ADAT input clock: 0 = master, 1 = slave.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_clock_mode(core: *mut FfiCore, mode: u8) -> u8 {
    status(core, |c| c.set_adat_input_clock_mode(mode))
}

/// Master-mode input rate for I2S and ADAT: 0 = 44.1k, 1 = 48k, 2 = 96k.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_rate(core: *mut FfiCore, index: u8) -> bool {
    flag(core, |c| c.set_input_rate(index))
}

/// The rate the pipeline runs at now, Hz (0 if unknown).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_input_rate(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.fetch_input_rate().unwrap_or(0))
}

/// I2S input clock: 0 = master, 1 = slave (applied by the device later).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_clock_mode(core: *mut FfiCore, mode: u8) -> bool {
    flag(core, |c| c.set_i2s_clock_mode(mode))
}

/// `which`: 0 = I2S slave lock, 1 = ADAT input lock; false for another value.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_input_lock(
    core: *mut FfiCore,
    which: u8,
    out: *mut InputLockStatus,
) -> bool {
    fetch(core, out, |c| c.fetch_input_lock(which))
}

/// The ADAT output's status (all zero on RP2040).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_adat_out_status(core: *mut FfiCore, out: *mut AdatOutStatus) -> bool {
    fetch(core, out, |c| c.fetch_adat_out_status())
}

// ── DAC hardware mute, LG Sound Sync, UART / I2C control ──────────────

/// Send a DAC mute config; read it back with `dspi_fetch_dac_mute` ~150 ms later.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_dac_mute(
    core: *mut FfiCore,
    enabled: bool,
    active_low: bool,
    pin: u8,
    hold_ms: u16,
    release_ms: u16,
) -> bool {
    flag(core, |c| {
        c.set_dac_mute(enabled, active_low, pin, hold_ms, release_ms)
    })
}

/// Re-read the DAC mute config into the state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_dac_mute(core: *mut FfiCore) -> bool {
    flag(core, |c| c.fetch_dac_mute())
}

/// Pulse the DAC mute output for ~1 s. Returns 0 when started, 3 when the
/// feature is disabled, 255 if the device did not answer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_test_dac_mute(core: *mut FfiCore) -> u8 {
    status(core, |c| c.test_dac_mute())
}

/// LG Sound Sync on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_lg_sound_sync(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_lg_sound_sync(enabled))
}

/// LG Sound Sync's status.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_lg_status(core: *mut FfiCore, out: *mut LgStatus) -> bool {
    fetch(core, out, |c| c.fetch_lg_status())
}

/// Re-read both control interface configs and their status into the state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_ctrl_ifaces(core: *mut FfiCore) -> bool {
    flag(core, |c| c.fetch_ctrl_ifaces())
}

/// Send a UART config; read the outcome with `dspi_fetch_ctrl_ifaces` ~250 ms later.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_uart(core: *mut FfiCore, config: UartConfig) -> bool {
    flag(core, |c| c.set_uart((&config).into()))
}

/// Send an I2C config; read the outcome with `dspi_fetch_ctrl_ifaces` later.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2c(core: *mut FfiCore, config: I2cConfig) -> bool {
    flag(core, |c| c.set_i2c((&config).into()))
}

// ── Device notifications ──────────────────────────────────────────────

/// Register the callback the notification listener thread calls when
/// packets are waiting; NULL clears it, waiting for a call in progress. It runs
/// on that thread: it must only schedule `dspi_process_notifications` on the
/// GUI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_notify_callback(
    core: *mut FfiCore,
    // Spelled out: cbindgen renders `Option<NotifyCallback>` as a struct.
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_core(core, (), |h| {
        let listener = callback.map(|callback| {
            Arc::new(CWake {
                callback,
                user_data,
            }) as Arc<dyn NotifyListener>
        });
        h.core.set_notify_listener(listener);
    });
}

/// Apply the queued device notifications to the state. `out` receives what
/// changed (`NOTIFY_*` flags and the wire channels whose filters changed);
/// zeros for a null handle. `out` may be null.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_process_notifications(core: *mut FfiCore, out: *mut NotifyResult) {
    let result = with_core(core, NotifyResult::default(), |h| {
        NotifyResult::from(&h.core.process_notifications())
    });
    if !out.is_null() && out.is_aligned() {
        // SAFETY: not null, aligned, and points to a record; the C caller's
        // contract.
        unsafe { out.write(result) };
    }
}

// ── Spectrum analyser ─────────────────────────────────────────────────

/// Register the callback the analyser worker calls (on its own thread) when
/// the caps or the snapshot changed. It must only schedule work on the GUI
/// thread. NULL stops the calls; once this returns, no call is in progress.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_set_callback(
    core: *mut FfiCore,
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_core(core, (), |h| {
        let listener = callback.map(|callback| {
            Arc::new(CWake {
                callback,
                user_data,
            }) as Arc<dyn RtaListener>
        });
        h.core.rta_set_listener(listener);
    });
}

/// Probe and start polling the connected device's analyser (compatible
/// firmware only). Stops by itself on disconnect.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_start(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.rta_start());
}

/// What to analyse. A zero mask releases the analyser.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_set_request(core: *mut FfiCore, request: *const RtaRequest) {
    if request.is_null() || !request.is_aligned() {
        return;
    }
    with_core(core, (), |h| {
        // SAFETY: not null, aligned, and points to a record; the C caller's
        // contract.
        let request = unsafe { request.read() };
        h.core.rta_set_request((&request).into());
    });
}

/// The analyser's capabilities (`supported` false until probed or without
/// one, and for a null handle).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_get_caps(core: *mut FfiCore, out: *mut RtaCapsInfo) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    let caps = with_core(core, None, |h| {
        Some(RtaCapsInfo::from(&h.core.rta_get_caps()))
    });
    // SAFETY: not null, aligned, and points to a record; the C caller's contract.
    unsafe { out.write(caps.unwrap_or_default()) };
}

/// Copy the latest snapshot into `out` (the empty one for a null handle).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_get_snapshot(core: *mut FfiCore, out: *mut RtaSnapshot) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    let snapshot = with_core(core, None, |h| {
        Some(RtaSnapshot::from(&h.core.rta_get_snapshot()))
    });
    // SAFETY: not null, aligned, and points to a record; the C caller's contract.
    unsafe { out.write(snapshot.unwrap_or_default()) };
}

// ── Signal generator ──────────────────────────────────────────────────

/// The generator's caps and signal types. False on a USB error; true with
/// `supported` false when the firmware has no generator.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_fetch_caps(core: *mut FfiCore, out: *mut SiggenCaps) -> bool {
    fetch(core, out, |c| c.siggen_fetch_caps())
}

/// The config the generator runs (as the device sanitised it).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_get_config(core: *mut FfiCore, out: *mut SiggenConfig) -> bool {
    fetch(core, out, |c| c.siggen_get_config())
}

/// Stage a config; a running generator restarts with it. The device ACKs
/// even a config it refuses: read it back to check.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_set_config(core: *mut FfiCore, cfg: *const SiggenConfig) -> bool {
    if cfg.is_null() || !cfg.is_aligned() {
        return false;
    }
    flag(core, |c| {
        // SAFETY: not null, aligned, and points to a record; the C caller's
        // contract.
        let cfg = unsafe { cfg.read() };
        c.siggen_set_config((&cfg).into())
    })
}

/// SIGGEN_CTL_START / STOP (fade) / STOP_NOW. False if refused.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_control(core: *mut FfiCore, action: u16) -> bool {
    flag(core, |c| c.siggen_control(action))
}

/// The generator's state.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_get_status(core: *mut FfiCore, out: *mut SiggenStatus) -> bool {
    fetch(core, out, |c| c.siggen_get_status())
}

// ── Statistics ────────────────────────────────────────────────────────

/// One REQ_GET_STATUS u32 counter (STAT_*).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_stat(core: *mut FfiCore, which: u16, out: *mut u32) -> bool {
    fetch_value(core, out, |c| c.fetch_stat(which))
}

/// The audio buffers' fill statistics.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_buffer_stats(core: *mut FfiCore, out: *mut BufferStats) -> bool {
    fetch(core, out, |c| c.fetch_buffer_stats())
}

/// Restart the buffer watermarks.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_reset_buffer_stats(core: *mut FfiCore) -> bool {
    flag(core, |c| c.reset_buffer_stats())
}

/// The S/PDIF receiver's status.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_status(core: *mut FfiCore, out: *mut SpdifRxStatus) -> bool {
    fetch(core, out, |c| c.fetch_spdif_rx_status())
}

/// `out` receives 24 bytes of IEC 60958 channel status.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_channel_status(core: *mut FfiCore, out: *mut u8) -> bool {
    if out.is_null() {
        return false;
    }
    with_core(core, false, |h| {
        h.core
            .fetch_spdif_rx_channel_status()
            .is_ok_and(|b| copy_bytes(&b, out, CHANNEL_STATUS_LEN))
    })
}

/// GPIO of S/PDIF receiver `index`; false if it has none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_pin(core: *mut FfiCore, index: u8, out: *mut u8) -> bool {
    fetch_value(core, out, |c| c.fetch_spdif_rx_pin(index))
}

// ── Interrupt monitor ─────────────────────────────────────────────────

/// Copy the notification log lines after `after_id` into `buf` as
/// "time_ms<TAB>text<LF>" (ms since the Unix epoch), stopping before `buf`
/// would overflow. A line too long for `buf` even on its own is dropped.
/// Returns the id of the last line written or dropped (`after_id` if none);
/// call again from there to get the rest.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_monitor_read(
    core: *mut FfiCore,
    after_id: u64,
    buf: *mut c_char,
    buf_len: u32,
) -> u64 {
    let Some(limit) = (buf_len as usize).checked_sub(1) else {
        return after_id;
    };
    if buf.is_null() {
        return after_id;
    }
    with_core(core, after_id, |h| {
        let mut out = String::new();
        let mut last = after_id;
        for e in h.core.monitor_read(after_id).entries {
            let line = format!("{}\t{}\n", e.time_ms, e.text);
            if line.len() > limit {
                // It would never fit: a caller looping until the id stops
                // moving must not stop here for good.
                log::debug!("dspi-capi: dropped a {}-byte monitor line", line.len());
                last = e.id;
                continue;
            }
            if out.len().saturating_add(line.len()) > limit {
                break;
            }
            out.push_str(&line);
            last = e.id;
        }
        copy_out(&out, buf, buf_len);
        last
    })
}

/// Empty the notification log.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_monitor_clear(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.monitor_clear());
}

// ── Configuration and filter files ────────────────────────────────────

/// The device's configuration as a `.dspipreset` document. `links` = the
/// app's input-pair link mask. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_export(
    core: *mut FfiCore,
    name: *const c_char,
    app_version: *const c_char,
    links: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        c.config_export(c_text(name), c_text(app_version), links)
    })
}

/// Check a document and describe it, as JSON {ok, error, name, platform,
/// firmware, savedUtc}. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_inspect(text: *const c_char, buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&dspi_core::files::config_inspect(c_text(text)), buf, len)
    })
}

/// Apply a document (`options`: 1 = volumes, 2 = hardware I/O). Writes JSON
/// {ok, error, lines, clean, links (or null), hardwareWritten}. Returns the
/// length needed.
///
/// Don't retry on a short buffer: a reply too long means the write has
/// happened, and a second call applies it again. Give it a large enough buffer
/// the first time.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_import(
    core: *mut FfiCore,
    text: *const c_char,
    options: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| c.config_import(c_text(text), options))
}

/// Every channel's filters as a text file (`inputs` = live inputs to write).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_export(
    core: *mut FfiCore,
    inputs: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.filters_export(inputs))
}

/// What a filter file holds, for the channel picker (JSON).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_inspect(
    core: *mut FfiCore,
    text: *const c_char,
    inputs: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| c.filters_inspect(c_text(text), inputs))
}

/// Apply a filter file to the channels in `wires` (wire indices). Writes
/// JSON {ok, error, lines}. Returns the length needed.
///
/// Don't retry on a short buffer: a reply too long means the write has
/// happened, and a second call applies it again. Give it a large enough buffer
/// the first time.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_import(
    core: *mut FfiCore,
    text: *const c_char,
    wires: *const u8,
    count: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| {
        c.filters_import(c_text(text), c_bytes(wires, count))
    })
}

// ── Control surfaces ──────────────────────────────────────────────────

/// Read the whole control surfaces configuration from the device (bindings,
/// IR commands, groups, macros, display, aux). Returns false on a transfer
/// error; firmware without control surfaces reads as unsupported.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_fetch(core: *mut FfiCore) -> bool {
    flag(core, |c| c.cs_fetch())
}

/// The control surfaces configuration as JSON. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_snapshot(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| c.cs_snapshot())
}

/// Perform one control surfaces operation, given as JSON {op, index, ...}.
/// Writes JSON {ok, status, error, ...}. Returns the length needed.
///
/// Don't retry on a short buffer: a reply too long means the write has
/// happened, and a second call applies it again. Give it a large enough buffer
/// the first time.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_apply(
    core: *mut FfiCore,
    op: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.cs_apply(c_text(op)))
}

/// GPIOs held by live control surface bindings: fills `pins` and `slots`
/// (up to `max` each) and returns how many there are.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_pin_uses(
    core: *mut FfiCore,
    pins: *mut u8,
    slots: *mut u8,
    max: u32,
) -> u32 {
    with_core(core, 0, |h| {
        let uses = h.core.cs_pin_uses();
        if !pins.is_null() && !slots.is_null() {
            for (i, u) in uses.iter().take(max as usize).enumerate() {
                // SAFETY: both arrays hold `max` bytes, the C caller's
                // contract, and i < max.
                unsafe {
                    pins.add(i).write(u.pin);
                    slots.add(i).write(u.slot);
                }
            }
        }
        u32::try_from(uses.len()).unwrap_or(u32::MAX)
    })
}

// ── Bootloader on the bus ─────────────────────────────────────────────

/// Boards in BOOTSEL (Raspberry Pi's ROM loader, not DSPi) on the USB bus:
/// fills the RP2040 and RP2350 counts. Either pointer may be null.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_bootloader_counts(rp2040: *mut u32, rp2350: *mut u32) {
    let counts = guard(Default::default(), dspi_core::bootloader_counts);
    // SAFETY: a non-null, aligned pointer points to a u32; the C caller's
    // contract.
    unsafe {
        if !rp2040.is_null() && rp2040.is_aligned() {
            rp2040.write(counts.rp2040);
        }
        if !rp2350.is_null() && rp2350.is_aligned() {
            rp2350.write(counts.rp2350);
        }
    }
}

// ── Added: the demo device ────────────────────────────────────────────

/// Open the built-in demo device: a simulated RP2350 that answers reads and
/// writes and notifies. Replaces any open device. Follow it with
/// `dspi_fetch_all`, as after `dspi_select_device`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_demo(core: *mut FfiCore) -> bool {
    flag(core, |c| c.open_demo())
}

// ── Added: extension channel ──────────────────────────────────────────

/// What the extension channel offers, as probed on connect (no traffic):
/// JSON {ok, supported, hasPairMap, hasCoreVreg}. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_ext_status(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.ext_status()))
}

/// Read the live crossfeed pair map into `out`. `out_len` must be at least 4;
/// a shorter buffer is false with no traffic. Also false on firmware without
/// the extension.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_crossfeed_pair_map(
    core: *mut FfiCore,
    out: *mut u8,
    out_len: u32,
) -> bool {
    if out.is_null() || (out_len as usize) < PAIR_MAP_LEN {
        return false;
    }
    with_core(core, false, |h| {
        h.core
            .fetch_crossfeed_pair_map()
            .is_ok_and(|m| copy_bytes(&m, out, PAIR_MAP_LEN))
    })
}

/// Set the crossfeed pair map (`map_len` 4 bytes). Returns 0 on success, the
/// device's status byte if it refused, 0xFF otherwise: an invalid map (nothing
/// is sent), firmware without the extension, or no device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_pair_map(
    core: *mut FfiCore,
    map: *const u8,
    map_len: u32,
) -> u8 {
    status(core, |c| c.set_crossfeed_pair_map(c_bytes(map, map_len)))
}

/// The core voltage extension's status: JSON {ok, liveVreg, millivolts,
/// storedSel, defaultVreg, autoRaised, autoDisabled, hangCount, trialActive,
/// trialVreg, failedTrialVreg, failedTrialUptimeS, uptimeS}. Returns the
/// length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_core_vreg(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_core_vreg()))
}

/// Core voltage write: `op` 0 set (value = VREG code, 0 = build default),
/// 1 trial, 2 end trial, 3 auto (value 0/1), 4 clear. Out-of-range requests
/// are refused here with nothing sent. Returns 0 on success, the device's
/// status byte if it refused, 0xFF otherwise. Read the status back with
/// `dspi_fetch_core_vreg`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_op(core: *mut FfiCore, op: u8, value: u8) -> u8 {
    status(core, |c| c.core_vreg_op(op, value))
}

// ── Added: names ──────────────────────────────────────────────────────

/// The user's name for input source `source` ("" when unnamed), cut to fit
/// `out_buf`; 32 bytes always fit. False on firmware without source names.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_input_source_name(
    core: *mut FfiCore,
    source: u8,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    if out_buf.is_null() || buf_len == 0 {
        return false;
    }
    with_core(core, false, |h| {
        h.core
            .get_input_source_name(source)
            .is_ok_and(|n| copy_out(&n, out_buf, buf_len))
    })
}

/// Name input source `source`; "" clears it. At most 31 bytes of UTF-8. False
/// for a null or invalid string (nothing is sent), and as the device refuses.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_source_name(
    core: *mut FfiCore,
    source: u8,
    name: *const c_char,
) -> bool {
    let Some(name) = c_str(name) else {
        return false;
    };
    flag(core, |c| c.set_input_source_name(source, name.to_owned()))
}

/// The device's own name ("" when unnamed), cut to fit `out_buf`; 32 bytes
/// always fit. False on firmware without device names.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_device_name(
    core: *mut FfiCore,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    if out_buf.is_null() || buf_len == 0 {
        return false;
    }
    with_core(core, false, |h| {
        h.core
            .get_device_name()
            .is_ok_and(|n| copy_out(&n, out_buf, buf_len))
    })
}

/// Rename the device; "" clears it. At most 31 bytes of UTF-8. False for a
/// null or invalid string (nothing is sent), and as the device refuses.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_device_name(core: *mut FfiCore, name: *const c_char) -> bool {
    let Some(name) = c_str(name) else {
        return false;
    };
    flag(core, |c| c.set_device_name(name.to_owned()))
}

// ── Added: diagnostics ────────────────────────────────────────────────

/// The output latency's status: JSON {ok, raw, fields: [{name, value}]}.
/// Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_latency(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_output_latency()))
}

/// The output latency options: JSON {ok, value: [{id, streamOnly, available,
/// latencyUs, batchUs, effectiveUs}]}. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_latency_options(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.fetch_output_latency_options())
    })
}

/// Request output latency option `id` (1..=7). The device applies it later.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_latency(core: *mut FfiCore, id: u8) -> bool {
    flag(core, |c| c.set_output_latency(id))
}

/// The clock diagnostics packet: JSON {ok, available, raw, fields: [{name,
/// value}]}; `available` false on firmware built without them. Returns the
/// length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_clock_diag(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_clock_diag()))
}

// -- WP CG12 --

/// The output rings' health: JSON {ok, targetFrames, targetUs, batchFrames,
/// underrunEpisodes, refusedBlocks, lowestFill, worstInputGapUs, worstUsbGapUs,
/// overBudgetSteps}, with `lowestFill` null before the first write. On firmware
/// without the status, {"ok":true,"value":null}: show no section. Returns the
/// length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_ring_health(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_ring_health()))
}

/// The live outputs' fill, on the ring's scale where the firmware has one:
/// JSON {ok, value: [{fillPct, minPct, maxPct, band}]}, `band` being "good",
/// "warn" or "bad". Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_fills(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_output_fills()))
}

// ── Added: firmware ───────────────────────────────────────────────────

/// Check a `.uf2` image: JSON {ok, valid, problem, familyId, platform,
/// numBlocks, payloadBytes, firmwareVersion}. `valid` is false for a null or
/// malformed image. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_uf2_inspect(
    image: *const u8,
    image_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let info = dspi_core::uf2_inspect(c_bytes(image, image_len));
        copy_text(&json_reply(Ok(info)), buf, len)
    })
}

/// Mounted bootloader drives: JSON {ok, value: [{path, platform, info}]}.
/// Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_bootloader_drives(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&json_reply(Ok(dspi_core::bootloader_drives())), buf, len)
    })
}

/// Write a validated image to the bootloader drive at `drive_path`, whose
/// `platform` comes from `dspi_bootloader_drives` (0 RP2040, 1 RP2350). The
/// image and the drive are checked before any byte goes out. Replies in JSON
/// {ok} or {ok, code, error}, and returns the length needed. `ok` means only
/// that every byte reached the drive: confirm with
/// `dspi_firmware_version_matches` once the DSPi is back.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_flash_to_drive(
    image: *const u8,
    image_len: u32,
    drive_path: *const c_char,
    platform: i8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let drive = BootloaderDrive {
            path: c_text(drive_path),
            platform,
            info: String::new(),
        };
        let done = dspi_core::flash_to_drive(c_bytes(image, image_len), drive);
        copy_text(&json_reply(done.map(|()| serde_json::json!({}))), buf, len)
    })
}

/// 1 if the open device runs firmware `version` (a UF2's `firmwareVersion`),
/// 0 if not, -1 with no device open or for a null or invalid string.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_version_matches(core: *mut FfiCore, version: *const c_char) -> i8 {
    let Some(version) = c_str(version) else {
        return -1;
    };
    with_core(core, -1, |h| {
        match h.core.firmware_version_matches(version) {
            Ok(same) => i8::from(same),
            Err(_) => -1,
        }
    })
}

#[cfg(test)]
#[path = "fns_misc/tests_wp_q2b.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_q2b;

// -- WP CG12 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "fns_misc/tests_wp_cg12.rs"]
mod tests_wp_cg12;
