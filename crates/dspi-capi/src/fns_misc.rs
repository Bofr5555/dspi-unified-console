//! C functions for everything around the parameters: the hardware
//! configuration, callbacks and notifications, the monitor log, the analyser,
//! the signal generator and statistics, files, control surfaces, the
//! extension channel, names, diagnostics, firmware, the demo device and the
//! core-voltage calibration. Each wraps one [`DspiCore`] method or one of
//! dspi-core's free functions, by the rules in `fns_device`.
//!
//! # Text replies
//!
//! A function with `buf` and `len` writes a NUL-terminated UTF-8 reply and
//! returns the bytes it needs, NUL included. When that is more than `len`,
//! nothing is written: call again with a buffer that large (a null `buf` asks
//! for the length). Those that change something do so on every call, whatever
//! the buffer; their replies are short. 0 means a null handle (or a panic).
//!
//! Most replies are JSON: a record's fields (camelCase) next to
//! `"ok": true`, a list or a plain value as `"value"`, and a failure as
//! `{"ok": false, "code": <kind>, "error": <sentence>}`. The kinds:
//! `disconnected`, `transfer`, `refused`, `invalid`, `unsupported`,
//! `read_only`, `locked`, `denied`, `link`, `io`, `autoeq`, `firmware_guard`,
//! `overvolt_word`. A document export replies the document itself, or that
//! failure.

#[cfg(test)]
use std::ffi::CStr;
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::sync::Arc;

use dspi_core::{
    BootloaderDrive, DeviceEvents, DspiCore, DspiError, FirmwareGuardBoard, NotifyListener,
    PairMap, ParamValue, Result, RtaListener, VregDemoEvent,
};
use serde_json::{Map, Value, json};

use crate::fns_device::{
    arg, c_str, copy_out, copy_text, fetch_value, flag, name_into, status, text,
};
use crate::guard;
use crate::types::*;
use crate::{FfiCore, with_core};

mod autoeq;
mod link_text;
// -- WP P0M --
mod events;
// -- WP P0E --
mod diag;
// -- WP FWX --
mod ext_diag;
// -- WP SW1 --
mod audio_link;

// ── Replies ──────────────────────────────────────────────────────────

/// The `code` of a failure reply (module docs).
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
        DspiError::AutoEq { .. } => "autoeq",
        DspiError::FirmwareGuardBlocked { .. } => "firmware_guard",
        DspiError::OvervoltWordRequired { .. } => "overvolt_word",
    }
}

/// A result as a JSON reply (module docs). The records replied this way have
/// no field named `ok`.
fn json_reply<T: serde::Serialize>(result: Result<T>) -> String {
    let encoded = result.and_then(|v| {
        serde_json::to_value(v).map_err(|e| DspiError::Io {
            msg: format!("the reply could not be encoded: {e}"),
        })
    });
    let reply = match encoded {
        Ok(Value::Object(fields)) => {
            let mut all = Map::new();
            all.insert("ok".to_owned(), Value::Bool(true));
            all.extend(fields);
            Value::Object(all)
        }
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(e) => json!({ "ok": false, "code": error_code(&e), "error": e.to_string() }),
    };
    reply.to_string()
}

/// A JSON reply for a call that answers nothing but success.
fn done_reply(result: Result<()>) -> String {
    json_reply(result.map(|()| Map::new()))
}

/// A document, or the failure reply in its place.
fn document_reply(result: Result<String>) -> String {
    result.unwrap_or_else(|e| json_reply::<()>(Err(e)))
}

/// An optional string argument: null is `None`; bytes that are not UTF-8 are
/// refused.
fn optional_text(p: *const c_char, name: &str) -> Result<Option<String>> {
    if p.is_null() {
        return Ok(None);
    }
    arg(p, name).map(|s| Some(s.to_owned()))
}

/// `len` bytes at `p`, copied; nothing for a null pointer.
fn c_bytes(p: *const u8, len: u32) -> Vec<u8> {
    if p.is_null() {
        return Vec::new();
    }
    let len = usize::try_from(len).unwrap_or(0);
    // SAFETY: a non-null buffer holds `len` bytes; the C caller's contract.
    unsafe { std::slice::from_raw_parts(p, len) }.to_vec()
}

/// Read one record the caller passes in: `None` for a null or misaligned
/// pointer.
fn read_in<T: Copy>(p: *const T) -> Option<T> {
    if p.is_null() || !p.is_aligned() {
        return None;
    }
    // SAFETY: non-null and aligned, and the caller's record lives for the
    // call; the C caller's contract.
    Some(unsafe { p.read() })
}

/// Store a record at `out` when it is non-null and aligned; otherwise
/// nothing.
fn store<T>(out: *mut T, value: T) {
    if !out.is_null() && out.is_aligned() {
        // SAFETY: non-null and aligned, pointing at the caller's record; the
        // C caller's contract.
        unsafe { out.write(value) };
    }
}

/// A function with no handle and a JSON reply: the bytes needed, 0 for a
/// panic.
fn free_text(buf: *mut c_char, len: u32, f: impl FnOnce() -> String) -> u32 {
    guard(0, || copy_text(&f(), buf, len))
}

// ── Callbacks ────────────────────────────────────────────────────────

/// A C wake-up callback, as the notification and analyser listeners.
struct CWake {
    callback: extern "C" fn(user_data: *mut c_void),
    user_data: *mut c_void,
}

// SAFETY: the callback is called on the core's threads, which its
// documentation tells the caller; `user_data` is only handed back to it.
unsafe impl Send for CWake {}
// SAFETY: as for Send.
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

/// The listener for a C callback; `None` unsets it.
fn wake(
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) -> Option<Arc<CWake>> {
    callback.map(|callback| {
        Arc::new(CWake {
            callback,
            user_data,
        })
    })
}

/// A C hot-plug callback, as device events: 0 arrived, 1 departed.
struct CHotplug {
    callback: extern "C" fn(event: u8, serial: *const c_char, user_data: *mut c_void),
    user_data: *mut c_void,
}

// SAFETY: as for CWake: the poll's thread is the caller's, and `user_data`
// is only handed back.
unsafe impl Send for CHotplug {}
// SAFETY: as for Send.
unsafe impl Sync for CHotplug {}

impl CHotplug {
    /// Call back with `serial` as a C string, up to any NUL in it.
    fn fire(&self, event: u8, serial: &str) {
        let printable = serial.split('\0').next().unwrap_or_default();
        let Ok(serial) = CString::new(printable) else {
            return;
        };
        (self.callback)(event, serial.as_ptr(), self.user_data);
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

// ── Output pins and the output configuration ─────────────────────────

/// Put output `slot` on GPIO `pin`. Returns a `PIN_CONFIG_*` status, 0xFF
/// when it could not be asked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_pin(core: *mut FfiCore, slot: u8, pin: u8) -> u8 {
    status(core, |c| c.set_output_pin(slot, pin))
}

/// The GPIO of output `slot`, 0xFF on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_pin(core: *mut FfiCore, slot: u8) -> u8 {
    with_core(core, 0xFF, |h| {
        h.core.fetch_output_pin(slot).unwrap_or(0xFF)
    })
}

/// Keep the output configuration with each preset (1) or once for all (0).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_config_mode(core: *mut FfiCore, mode: u8) -> bool {
    flag(core, |c| c.set_output_config_mode(mode))
}

/// Store the output configuration in flash. Returns the device's status,
/// 0xFF when it could not be asked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_output_config(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::save_output_config)
}

/// Reboot the device into its USB bootloader. No answer comes back, so true
/// means only that the request went out.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_enter_bootloader(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::enter_bootloader)
}

/// Hold the device for this console (DSPi Link; at once elsewhere) for up to
/// `timeout_ms`, giving `reason` to the others. JSON `{ok}` or a failure
/// (`locked` when another console holds it). Returns the bytes needed; 0 for
/// a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_device_lock_acquire(
    core: *mut FfiCore,
    reason: *const c_char,
    timeout_ms: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let held =
            arg(reason, "reason").and_then(|why| c.device_lock_acquire(why.to_owned(), timeout_ms));
        done_reply(held)
    })
}

/// Give back the lock `dspi_device_lock_acquire` took. Nothing if not held.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_device_lock_release(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.device_lock_release());
}

/// Core 1's mode, -1 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_core1_mode(core: *mut FfiCore) -> i8 {
    with_core(core, -1, |h| {
        h.core
            .fetch_core1_mode()
            .ok()
            .and_then(|mode| i8::try_from(mode).ok())
            .unwrap_or(-1)
    })
}

/// Whether enabling `output` would collide on Core 1: 1 yes, 0 no, -1 on
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_check_core1_conflict(core: *mut FfiCore, output: u8) -> i8 {
    with_core(core, -1, |h| {
        h.core.check_core1_conflict(output).map_or(-1, i8::from)
    })
}

// ── Hot-plug ─────────────────────────────────────────────────────────

/// Set the callback `dspi_poll_hotplug` reports arrivals (0) and departures
/// (1) with, on the polling thread; null unsets it. The serial is valid
/// during the call only.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_hotplug_callback(
    core: *mut FfiCore,
    callback: Option<extern "C" fn(event: u8, serial: *const c_char, user_data: *mut c_void)>,
    user_data: *mut c_void,
) -> bool {
    with_core(core, false, |h| {
        let events = callback.map(|callback| -> Arc<dyn DeviceEvents> {
            Arc::new(CHotplug {
                callback,
                user_data,
            })
        });
        h.core.set_hotplug_callback(events);
        true
    })
}

/// Look for devices that came or went, and report them to the callback.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_poll_hotplug(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.poll_hotplug());
}

// ── Hardware configuration (status: PIN_CONFIG_*, 0xFF) ──────────────

/// Set the kind of output `slot` (I2S, S/PDIF, ...).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_type(core: *mut FfiCore, slot: u8, kind: u8) -> u8 {
    status(core, |c| c.set_output_type(slot, kind))
}

/// Put an I2S bit clock (`role`) on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_bck_pin(core: *mut FfiCore, role: u8, pin: u8) -> u8 {
    status(core, |c| c.set_i2s_bck_pin(role, pin))
}

/// Choose how the I2S clock pins are laid out.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_clock_pin_mode(core: *mut FfiCore, mode: u8) -> u8 {
    status(core, |c| c.set_i2s_clock_pin_mode(mode))
}

/// Turn the master clock output on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_mck_enabled(enabled))
}

/// Put the master clock on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_mck_pin(pin))
}

/// Set the master clock's multiple of the sample rate.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_mck_multiplier(core: *mut FfiCore, mult: u8) -> u8 {
    status(core, |c| c.set_mck_multiplier(mult))
}

/// Turn the ADAT output on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_out_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_adat_out_enabled(enabled))
}

/// Put the ADAT output on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_out_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_adat_out_pin(pin))
}

/// Put S/PDIF receiver `index` on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_spdif_rx_pin(core: *mut FfiCore, index: u8, pin: u8) -> u8 {
    status(core, |c| c.set_spdif_rx_pin(index, pin))
}

/// Turn S/PDIF input `index` on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_spdif_input_enabled(core: *mut FfiCore, index: u8, enabled: bool) -> u8 {
    status(core, |c| c.set_spdif_input_enabled(index, enabled))
}

/// Put I2S input `pair` on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_rx_pin(core: *mut FfiCore, pair: u8, pin: u8) -> u8 {
    status(core, |c| c.set_i2s_rx_pin(pair, pin))
}

/// Set how many channels the I2S input carries.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_input_channels(core: *mut FfiCore, channels: u8) -> u8 {
    status(core, |c| c.set_i2s_input_channels(channels))
}

/// Turn the ADAT input on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_enabled(core: *mut FfiCore, enabled: bool) -> u8 {
    status(core, |c| c.set_adat_input_enabled(enabled))
}

/// Put the ADAT input on GPIO `pin`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_pin(core: *mut FfiCore, pin: u8) -> u8 {
    status(core, |c| c.set_adat_input_pin(pin))
}

/// Choose where the ADAT input takes its clock from.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_adat_input_clock_mode(core: *mut FfiCore, mode: u8) -> u8 {
    status(core, |c| c.set_adat_input_clock_mode(mode))
}

/// Choose the input sample rate by its index.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_rate(core: *mut FfiCore, index: u8) -> bool {
    flag(core, |c| c.set_input_rate(index))
}

/// The input sample rate in Hz, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_input_rate(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.fetch_input_rate().unwrap_or(0))
}

/// Choose whether the I2S clocks are driven or followed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2s_clock_mode(core: *mut FfiCore, mode: u8) -> bool {
    flag(core, |c| c.set_i2s_clock_mode(mode))
}

/// The lock state of digital input `which` into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_input_lock(
    core: *mut FfiCore,
    which: u8,
    out: *mut InputLockStatus,
) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_input_lock(which).map(|s| InputLockStatus::from(&s))
    })
}

/// The ADAT output's state into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_adat_out_status(core: *mut FfiCore, out: *mut AdatOutStatus) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_adat_out_status().map(|s| AdatOutStatus::from(&s))
    })
}

/// Configure the DAC mute line: on or off, its polarity, its GPIO, and how
/// long it holds before and after a change, in ms.
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

/// Read the DAC mute configuration into the mirror.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_dac_mute(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::fetch_dac_mute)
}

/// Pulse the DAC mute line once. Returns the device's status, 0xFF when it
/// could not be asked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_test_dac_mute(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::test_dac_mute)
}

/// Turn LG Sound Sync on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_lg_sound_sync(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_lg_sound_sync(enabled))
}

/// LG Sound Sync's state into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_lg_status(core: *mut FfiCore, out: *mut LgStatus) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_lg_status().map(|s| LgStatus::from(&s))
    })
}

/// Read the UART and I2C control interfaces into the mirror.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_ctrl_ifaces(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::fetch_ctrl_ifaces)
}

/// Configure the UART control interface.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_uart(core: *mut FfiCore, config: UartConfig) -> bool {
    flag(core, |c| c.set_uart(dspi_core::UartConfig::from(&config)))
}

/// Configure the I2C control interface.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_i2c(core: *mut FfiCore, config: I2cConfig) -> bool {
    flag(core, |c| c.set_i2c(dspi_core::I2cConfig::from(&config)))
}

// ── Notifications ────────────────────────────────────────────────────

/// Set the callback that says notifications are waiting. It runs on the
/// reader's thread and must only schedule `dspi_process_notifications`. Null
/// unsets it; once that returns, the old one is not running.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_notify_callback(
    core: *mut FfiCore,
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_core(core, (), |h| {
        let listener = wake(callback, user_data).map(|w| -> Arc<dyn NotifyListener> { w });
        h.core.set_notify_listener(listener);
    });
}

/// Apply the waiting notifications to the mirror and say what changed in
/// `out` (may be null). A null handle gives the empty result.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_process_notifications(core: *mut FfiCore, out: *mut NotifyResult) {
    let result = with_core(core, NotifyResult::default(), |h| {
        NotifyResult::from(&h.core.process_notifications())
    });
    store(out, result);
}

// ── The analyser ─────────────────────────────────────────────────────

/// Set the callback that says new analyser data is waiting, on the
/// analyser's thread; null unsets it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_set_callback(
    core: *mut FfiCore,
    callback: Option<extern "C" fn(user_data: *mut c_void)>,
    user_data: *mut c_void,
) {
    with_core(core, (), |h| {
        let listener = wake(callback, user_data).map(|w| -> Arc<dyn RtaListener> { w });
        h.core.rta_set_listener(listener);
    });
}

/// Start the analyser's worker for the open device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_start(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.rta_start());
}

/// Ask for an analysis; a zero mask releases the analyser. A null request
/// does nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_set_request(core: *mut FfiCore, request: *const RtaRequest) {
    let Some(request) = read_in(request) else {
        return;
    };
    with_core(core, (), |h| {
        h.core
            .rta_set_request(dspi_core::RtaRequest::from(&request));
    });
}

/// `dspi_rta_set_request` for a view that is on screen: with `watching`, a
/// zero mask keeps showing what another console has the analyser doing
/// instead of releasing it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_set_request_watching(
    core: *mut FfiCore,
    request: *const RtaRequest,
    watching: bool,
) {
    let Some(request) = read_in(request) else {
        return;
    };
    with_core(core, (), |h| {
        let mut wanted = dspi_core::RtaRequest::from(&request);
        wanted.watching = watching;
        h.core.rta_set_request(wanted);
    });
}

/// The analyser's capabilities into `out`; "not supported" for a null
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_get_caps(core: *mut FfiCore, out: *mut RtaCapsInfo) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    let caps = with_core(core, RtaCapsInfo::default(), |h| {
        RtaCapsInfo::from(&h.core.rta_get_caps())
    });
    store(out, caps);
}

/// The latest analysis into `out`; empty for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_get_snapshot(core: *mut FfiCore, out: *mut RtaSnapshot) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    let snapshot = with_core(core, RtaSnapshot::default(), |h| {
        RtaSnapshot::from(&h.core.rta_get_snapshot())
    });
    store(out, snapshot);
}

/// Who chose the analyser's configuration, into `out`; "this console drives"
/// for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_get_ownership(core: *mut FfiCore, out: *mut RtaOwnership) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    let owner = with_core(core, RtaOwnership::default(), |h| {
        RtaOwnership::from(&h.core.rta_get_ownership())
    });
    store(out, owner);
}

/// Make this console the one that drives the analyser again.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_rta_take_control(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.rta_take_control());
}

// ── Signal generator and statistics ──────────────────────────────────

/// The signal generator's capabilities into `out`. True with `supported`
/// false on firmware without one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_fetch_caps(core: *mut FfiCore, out: *mut SiggenCaps) -> bool {
    fetch_value(core, out, |c| {
        c.siggen_fetch_caps().map(|caps| SiggenCaps::from(&caps))
    })
}

/// The signal generator's configuration into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_get_config(core: *mut FfiCore, out: *mut SiggenConfig) -> bool {
    fetch_value(core, out, |c| {
        c.siggen_get_config().map(|cfg| SiggenConfig::from(&cfg))
    })
}

/// Configure the signal generator. False for a null `cfg`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_set_config(core: *mut FfiCore, cfg: *const SiggenConfig) -> bool {
    let Some(cfg) = read_in(cfg) else {
        return false;
    };
    flag(core, |c| {
        c.siggen_set_config(dspi_core::SiggenConfig::from(&cfg))
    })
}

/// Start, stop or otherwise drive the signal generator (`action`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_control(core: *mut FfiCore, action: u16) -> bool {
    flag(core, |c| c.siggen_control(action))
}

/// The signal generator's state into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_siggen_get_status(core: *mut FfiCore, out: *mut SiggenStatus) -> bool {
    fetch_value(core, out, |c| {
        c.siggen_get_status().map(|s| SiggenStatus::from(&s))
    })
}

/// One statistic (`STAT_*`) into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_stat(core: *mut FfiCore, which: u16, out: *mut u32) -> bool {
    fetch_value(core, out, |c| c.fetch_stat(which))
}

/// The buffer fill levels into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_buffer_stats(core: *mut FfiCore, out: *mut BufferStats) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_buffer_stats().map(|s| BufferStats::from(&s))
    })
}

// -- WP CS1 --
/// The device's USB error counters (`REQ_GET_USB_ERROR_STATS`), in JSON:
/// `{ok, total, crc, bitstuff, rxOverflow, rxTimeout, dataSeq}`, or a
/// failure. Current firmware reports zeros.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_usb_error_stats(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_usb_error_stats()))
}

/// Reset the buffer statistics' watermarks.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_reset_buffer_stats(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::reset_buffer_stats)
}

/// The S/PDIF receiver's state into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_status(core: *mut FfiCore, out: *mut SpdifRxStatus) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_spdif_rx_status().map(|s| SpdifRxStatus::from(&s))
    })
}

/// The bytes of the IEC 60958 channel status block.
const CHANNEL_STATUS_BYTES: usize = 24;

/// The received S/PDIF channel status into `out`, which holds 24 bytes;
/// bytes the device did not send are 0.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_channel_status(core: *mut FfiCore, out: *mut u8) -> bool {
    if out.is_null() {
        return false;
    }
    with_core(core, false, |h| {
        let Ok(block) = h.core.fetch_spdif_rx_channel_status() else {
            return false;
        };
        let padded = block.into_iter().chain(std::iter::repeat(0));
        for (i, byte) in padded.take(CHANNEL_STATUS_BYTES).enumerate() {
            // SAFETY: `out` holds 24 bytes (the C caller's contract) and
            // `i < 24`.
            unsafe { out.add(i).write(byte) };
        }
        true
    })
}

/// The GPIO of S/PDIF receiver `index` into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_spdif_rx_pin(core: *mut FfiCore, index: u8, out: *mut u8) -> bool {
    fetch_value(core, out, |c| c.fetch_spdif_rx_pin(index))
}

// ── The monitor log ──────────────────────────────────────────────────

/// The log entries after `after_id` into `buf`, one per line as
/// `<ms since the epoch>\t<text>\n`, as many whole lines as fit. Returns the
/// id to pass next time: the last entry written, or past entries too long
/// for the buffer at all, which are dropped. With nothing new, a null buffer,
/// a length of 0 or a null handle it returns `after_id`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_monitor_read(
    core: *mut FfiCore,
    after_id: u64,
    buf: *mut c_char,
    buf_len: u32,
) -> u64 {
    if buf.is_null() || buf_len == 0 {
        return after_id;
    }
    with_core(core, after_id, |h| {
        let batch = h.core.monitor_read(after_id);
        let room = usize::try_from(buf_len).map_or(usize::MAX, |n| n.saturating_sub(1));
        let mut lines = String::new();
        let mut next = after_id;
        let mut stopped = false;
        for entry in &batch.entries {
            let flat: String = entry
                .text
                .chars()
                .map(|ch| if ch == '\n' || ch == '\0' { ' ' } else { ch })
                .collect();
            let line = format!("{}\t{flat}\n", entry.time_ms);
            if line.len() <= room {
                if lines.len().saturating_add(line.len()) > room {
                    stopped = true;
                    break;
                }
                lines.push_str(&line);
            }
            next = entry.id;
        }
        if !stopped {
            next = next.max(batch.last_id);
        }
        copy_out(&lines, buf, buf_len);
        next
    })
}

/// Empty the monitor log.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_monitor_clear(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.monitor_clear());
}

// ── Files ────────────────────────────────────────────────────────────

/// The device's whole configuration as a `.dspipreset` document named
/// `name`, written by `app_version`, with the input links `links`; or a JSON
/// failure. Returns the bytes needed.
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
        let doc = arg(name, "name").and_then(|name| {
            let version = arg(app_version, "app_version")?;
            c.config_export(name.to_owned(), version.to_owned(), links)
        });
        document_reply(doc)
    })
}

/// What a configuration document holds, in JSON, without a device. Returns
/// the bytes needed, 0 only for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_inspect(text: *const c_char, buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || match arg(text, "text") {
        Ok(doc) => dspi_core::files::config_inspect(doc.to_owned()),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

/// Apply a configuration document to the device. `options`: bit 0 the
/// volumes too, bit 1 the hardware configuration too. JSON reply; the device
/// has changed whatever the buffer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_config_import(
    core: *mut FfiCore,
    text: *const c_char,
    options: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| match arg(text, "text") {
        Ok(doc) => c.config_import(doc.to_owned(), options),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

/// The filters of the device's `inputs` inputs and every output as a filter
/// document, or a JSON failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_export(
    core: *mut FfiCore,
    inputs: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| document_reply(c.filters_export(inputs)))
}

/// What a filter document holds that this device can use, in JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_inspect(
    core: *mut FfiCore,
    text: *const c_char,
    inputs: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| match arg(text, "text") {
        Ok(doc) => c.filters_inspect(doc.to_owned(), inputs),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

/// Apply a filter document's channels to the `count` wire channels at
/// `wires`. JSON reply; the device has changed whatever the buffer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filters_import(
    core: *mut FfiCore,
    text: *const c_char,
    wires: *const u8,
    count: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    self::text(core, buf, len, |c| match arg(text, "text") {
        Ok(doc) => c.filters_import(doc.to_owned(), c_bytes(wires, count)),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

// ── Control surfaces ─────────────────────────────────────────────────

/// Read the control surface configuration from the device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_fetch(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::cs_fetch)
}

/// The control surface configuration last read, as JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_snapshot(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, DspiCore::cs_snapshot)
}

/// Run one control surface operation, given as JSON; the JSON reply carries
/// an `error` when it failed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_apply(
    core: *mut FfiCore,
    op: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| match arg(op, "op") {
        Ok(op) => c.cs_apply(op.to_owned()),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

/// The GPIOs live bindings hold: up to `max` of them into `pins`, with the
/// binding's slot at the same index of `slots` (either may be null). Returns
/// how many there are in all; 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_pin_uses(
    core: *mut FfiCore,
    pins: *mut u8,
    slots: *mut u8,
    max: u32,
) -> u32 {
    with_core(core, 0, |h| {
        let uses = h.core.cs_pin_uses();
        let room = usize::try_from(max).unwrap_or(usize::MAX);
        for (i, used) in uses.iter().take(room).enumerate() {
            // SAFETY: each non-null array holds `max` bytes (the C caller's
            // contract) and `i < max`.
            unsafe {
                if !pins.is_null() {
                    pins.add(i).write(used.pin);
                }
                if !slots.is_null() {
                    slots.add(i).write(used.slot);
                }
            }
        }
        u32::try_from(uses.len()).unwrap_or(u32::MAX)
    })
}

// ── Bootloader boards and the demo device ────────────────────────────

/// How many RP2040 and RP2350 boards are in their USB bootloader. Either
/// pointer may be null.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_bootloader_counts(rp2040: *mut u32, rp2350: *mut u32) {
    let counts = guard(None, || Some(dspi_core::bootloader_counts()));
    if let Some(counts) = counts {
        store(rp2040, counts.rp2040);
        store(rp2350, counts.rp2350);
    }
}

/// Open the demo device, a simulated DSPi, in place of any open device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_demo(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::open_demo)
}

// ── The extension channel ────────────────────────────────────────────

/// What the extension channel offers on the open device, in JSON:
/// `{ok, supported, hasPairMap, hasCoreVreg}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_ext_status(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.ext_status()))
}

/// The crossfeed pair map, 4 bytes, into `out` of `out_len` bytes. False for
/// a buffer under 4 bytes, with nothing written.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_crossfeed_pair_map(
    core: *mut FfiCore,
    out: *mut u8,
    out_len: u32,
) -> bool {
    let fits = usize::try_from(out_len).is_ok_and(|n| n >= PairMap::SIZE);
    if out.is_null() || !fits {
        return false;
    }
    with_core(core, false, |h| {
        let Ok(map) = h.core.fetch_crossfeed_pair_map() else {
            return false;
        };
        let padded = map.into_iter().chain(std::iter::repeat(0));
        for (i, byte) in padded.take(PairMap::SIZE).enumerate() {
            // SAFETY: `out` holds `out_len` >= 4 bytes and `i < 4`.
            unsafe { out.add(i).write(byte) };
        }
        true
    })
}

/// Write the crossfeed pair map: `map_len` (4) bytes at `map`. Returns a
/// status byte; 0xFF for a null or short map without asking the device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_pair_map(
    core: *mut FfiCore,
    map: *const u8,
    map_len: u32,
) -> u8 {
    let long_enough = usize::try_from(map_len).is_ok_and(|n| n >= PairMap::SIZE);
    if map.is_null() || !long_enough {
        return 0xFF;
    }
    let bytes = c_bytes(map, map_len);
    status(core, |c| c.set_crossfeed_pair_map(bytes))
}

/// Whether `map` (`map_len` bytes, which must be 4) is a pair map the
/// firmware takes with `num_pairs` pairs.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_crossfeed_pair_map_valid(
    map: *const u8,
    map_len: u32,
    num_pairs: u8,
) -> bool {
    if map.is_null() {
        return false;
    }
    guard(false, || {
        dspi_core::crossfeed_pair_map_valid(c_bytes(map, map_len), num_pairs)
    })
}

/// The K340's pair map into `out` (4 bytes of `out_len`). False for a
/// buffer under 4 bytes.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_crossfeed_k340_pair_map(out: *mut u8, out_len: u32) -> bool {
    let fits = usize::try_from(out_len).is_ok_and(|n| n >= PairMap::SIZE);
    if out.is_null() || !fits {
        return false;
    }
    let PairMap(bytes) = PairMap::K340;
    // SAFETY: `out` holds `out_len` >= 4 bytes (the C caller's contract).
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    true
}

/// The output pair mask that goes with the K340's pair map.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_crossfeed_k340_pair_mask() -> u8 {
    PairMap::K340_MASK
}

/// The core voltage extension's status, in JSON (`millivolts`, `storedSel`,
/// ...), or a failure (`unsupported` without it).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_core_vreg(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_core_vreg()))
}

/// One raw core voltage operation. `word` is what the user typed (null:
/// nothing); raising above 1.20 V needs `OVERVOLT`. Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_op(
    core: *mut FfiCore,
    op: u8,
    value: u8,
    word: *const c_char,
) -> u8 {
    status(core, |c| {
        c.core_vreg_op(op, value, optional_text(word, "word")?)
    })
}

// ── Names ────────────────────────────────────────────────────────────

/// The user's name for input source `source`, read from the device, into
/// `out_buf`, cut to fit.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_input_source_name(
    core: *mut FfiCore,
    source: u8,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, |c| c.get_input_source_name(source))
}

/// Name input source `source`. False for a null or non-UTF-8 name, or one too
/// long for the device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_source_name(
    core: *mut FfiCore,
    source: u8,
    name: *const c_char,
) -> bool {
    flag(core, |c| {
        c.set_input_source_name(source, arg(name, "name")?.to_owned())
    })
}

/// The device's name, read from it, into `out_buf`, cut to fit.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_device_name(
    core: *mut FfiCore,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, DspiCore::get_device_name)
}

/// Name the device. False for a null or non-UTF-8 name, or one too long.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_device_name(core: *mut FfiCore, name: *const c_char) -> bool {
    flag(core, |c| c.set_device_name(arg(name, "name")?.to_owned()))
}

/// The input source's name as last read (no I/O), into `out_buf`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cached_input_source_name(
    core: *mut FfiCore,
    source: u8,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, |c| {
        c.cached_input_source_name(source)
    })
}

/// The device's name as last read (no I/O), into `out_buf`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cached_device_name(
    core: *mut FfiCore,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, DspiCore::cached_device_name)
}

/// Demo device: another console renames input source `target` (0xFF: the
/// device) to `name`, as a notification shows it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_demo_rename_elsewhere(
    core: *mut FfiCore,
    target: u8,
    name: *const c_char,
) -> bool {
    flag(core, |c| {
        c.demo_rename_elsewhere(target, arg(name, "name")?.to_owned())
    })
}

// ── The demo device's timings and fixtures ───────────────────────────

/// Demo device: how output latency changes behave (`mode`), how many reads
/// they take and how long the device goes quiet.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_demo_latency_timing(
    core: *mut FfiCore,
    mode: u8,
    reads: u8,
    blackout: u8,
) -> bool {
    flag(core, |c| c.demo_latency_timing(mode, reads, blackout))
}

/// Demo device: how many output latency changes it took. `u32::MAX` on
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_demo_latency_sets(core: *mut FfiCore) -> u32 {
    with_core(core, u32::MAX, |h| {
        h.core.demo_latency_sets().unwrap_or(u32::MAX)
    })
}

/// Demo device: the status its next output configuration save answers.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_demo_output_save_status(core: *mut FfiCore, status: u8) -> bool {
    flag(core, |c| c.demo_output_save_status(status))
}

/// Open a recorded device of wire format `wire` (read-only below the
/// current format and above it).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_demo_open_wire(core: *mut FfiCore, wire: u8) -> bool {
    flag(core, |c| c.open_demo_wire(wire))
}

/// How many writes the read-only mode has refused on this handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_read_only_refusals(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.read_only_refusals())
}

/// The open device's wire features (`WIRE_FEAT_*`); 0 without one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_wire_features(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.wire_features())
}

// -- WP QP11 --
/// Whether the connect probe found feature `name` on the open device (the
/// probe's names: `spdif_multi_input`, `adat_input`, `i2s_slave_clock`, ...;
/// `i2s_clock_pin_mode`: the parameter image carries the clock-pin mode).
/// False with no device, for a null name and for a name the probe lacks.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_feature_present(core: *mut FfiCore, name: *const c_char) -> bool {
    let Some(name) = c_str(name) else {
        return false;
    };
    with_core(core, false, |h| h.core.feature_present(name))
}

// -- WP QP16 --
/// The last call that failed on the calling thread, as its failure reply
/// (`{ok: false, code, error}`: the core's own sentence), or `{ok: true}`
/// when none failed since the last take. It is taken (cleared) only when the
/// reply fits in `len`. For a client that wants the reason after a `bool`
/// setter returned false: take it once before the call to clear an older one,
/// then again after. Takes no handle; never fails.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_take_failure(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        let reply = |e: Option<DspiError>| match e {
            Some(e) => json_reply::<()>(Err(e)),
            None => json!({ "ok": true }).to_string(),
        };
        let needed = copy_text(
            &reply(crate::fns_device::last_failure(false)),
            ptr::null_mut(),
            0,
        );
        if buf.is_null() || needed > len {
            return needed;
        }
        copy_text(&reply(crate::fns_device::last_failure(true)), buf, len)
    })
}

// ── Diagnostics ──────────────────────────────────────────────────────

/// The output latency the device reports, in JSON: `{ok, raw, fields}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_latency(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_output_latency()))
}

/// The output latency settings to choose from, in JSON (`value`: a list).
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

/// Choose an output latency setting by its id.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_latency(core: *mut FfiCore, id: u8) -> bool {
    flag(core, |c| c.set_output_latency(id))
}

/// The clock diagnostics, in JSON (`available` false on firmware without
/// them).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_clock_diag(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_clock_diag()))
}

/// The output ring's health, in JSON; `{ok, value: null}` when the device
/// does not report it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_ring_health(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_ring_health()))
}

/// Each live output's fill, in JSON (`value`: a list).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_output_fills(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_output_fills()))
}

/// The buffers and the output fills from one read, in JSON: `{ok, buffers,
/// fills}`; `fills` is null when they could not be read (keep the last).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_stats_tick(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.fetch_stats_tick()))
}

/// Start or stop keeping a copy of every notification for
/// `dspi_notify_tap_drain`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_notify_tap_set(core: *mut FfiCore, enabled: bool) {
    with_core(core, (), |h| h.core.notify_tap_set(enabled));
}

/// The notifications kept since the last drain, in JSON (`value`: a list of
/// `{packet, origin, own, lost, receivedMs}`). They are taken whatever the
/// buffer: when they do not fit, the next one kept is marked `lost`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_notify_tap_drain(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    with_core(core, 0, |h| {
        let kept = h.core.notify_tap_drain();
        let any = !kept.is_empty();
        let needed = copy_text(&json_reply(Ok(kept)), buf, len);
        if any && (buf.is_null() || needed > len) {
            h.core.notify_tap_note_dropped();
        }
        needed
    })
}

/// The open connection's generation: new on every open, 0 with none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_connection_generation(core: *mut FfiCore) -> u64 {
    with_core(core, 0, |h| h.core.connection_generation())
}

// ── Firmware ─────────────────────────────────────────────────────────

/// Check a UF2 image of `image_len` bytes, in JSON: `{ok, valid, problem,
/// familyId, platform, ...}`. A null image is an empty one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_uf2_inspect(
    image: *const u8,
    image_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || {
        json_reply(Ok(dspi_core::uf2_inspect(c_bytes(image, image_len))))
    })
}

/// The boards' bootloader drives that are mounted, in JSON (`value`: a list
/// of `{path, platform, info}`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_bootloader_drives(buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || json_reply(Ok(dspi_core::bootloader_drives())))
}

/// The drive record a caller describes by its path and platform.
fn drive_at(drive_path: *const c_char, platform: i8) -> Result<BootloaderDrive> {
    Ok(BootloaderDrive {
        path: arg(drive_path, "drive_path")?.to_owned(),
        platform,
        info: String::new(),
    })
}

/// Write a UF2 image to the bootloader drive at `drive_path` (`platform`: 0
/// RP2040, 1 RP2350). Blocks for the copy. JSON `{ok}` or a failure; an
/// invalid image never reaches the drive. Use the guarded call instead.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_flash_to_drive(
    image: *const u8,
    image_len: u32,
    drive_path: *const c_char,
    platform: i8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || {
        let written = drive_at(drive_path, platform)
            .and_then(|drive| dspi_core::flash_to_drive(c_bytes(image, image_len), drive));
        done_reply(written)
    })
}

/// Whether the open device runs firmware `version`: 1 yes, 0 no, -1 on
/// failure or for a null or non-UTF-8 version.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_version_matches(core: *mut FfiCore, version: *const c_char) -> i8 {
    let Some(version) = c_str(version) else {
        return -1;
    };
    with_core(core, -1, |h| {
        h.core
            .firmware_version_matches(version)
            .map_or(-1, i8::from)
    })
}

// ── Core voltage calibration ─────────────────────────────────────────

/// The calibration flow as it stands, in JSON (`phase`, `choices`,
/// `lastError`, ...). No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_view(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.core_vreg_cal_view())))
}

/// The calibration flow after moving it on (reads the device), in JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_poll(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.core_vreg_cal_poll())))
}

/// Open the calibration for review. Returns a status byte; the reason for a
/// refusal is the view's `lastError`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_open(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::core_vreg_cal_open)
}

/// Close the calibration. Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_close(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::core_vreg_cal_close)
}

/// Start a trial at VREG code `target` for `soak_s` seconds; `typed` is the
/// word for a raise above 1.20 V (null: none). Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_start(
    core: *mut FfiCore,
    target: u8,
    soak_s: u32,
    typed: *const c_char,
) -> u8 {
    status(core, |c| {
        c.core_vreg_cal_start(target, soak_s, optional_text(typed, "typed")?)
    })
}

/// Keep the trial's voltage for another `soak_s` seconds. Returns a status
/// byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_keep(core: *mut FfiCore, soak_s: u32) -> u8 {
    status(core, |c| c.core_vreg_cal_keep(soak_s))
}

/// Store the passed trial's voltage; `typed` as for the start. Returns a
/// status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_cal_commit(core: *mut FfiCore, typed: *const c_char) -> u8 {
    status(core, |c| {
        c.core_vreg_cal_commit(optional_text(typed, "typed")?)
    })
}

/// End the running trial. Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_trial_end(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::core_vreg_trial_end)
}

/// Store VREG code `value` directly (0: the build's default); `typed` as for
/// the start. Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_manual_set(
    core: *mut FfiCore,
    value: u8,
    typed: *const c_char,
) -> u8 {
    status(core, |c| {
        c.core_vreg_manual_set(value, optional_text(typed, "typed")?)
    })
}

/// Try VREG code `value` until the next reboot; `typed` as for the start.
/// Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_manual_trial(
    core: *mut FfiCore,
    value: u8,
    typed: *const c_char,
) -> u8 {
    status(core, |c| {
        c.core_vreg_manual_trial(value, optional_text(typed, "typed")?)
    })
}

/// Let the firmware raise the voltage by itself after hangs, or not.
/// Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_manual_auto(core: *mut FfiCore, on: bool) -> u8 {
    status(core, |c| c.core_vreg_manual_auto(on))
}

/// Clear the hang records. Returns a status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_manual_clear(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::core_vreg_manual_clear)
}

/// Demo device: something happens to it. `event`: 0 a hang, 1 a reboot, 2 a
/// power cycle, 3 `seconds` pass. Returns a status byte; 0xFF for another
/// event.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_vreg_demo_event(core: *mut FfiCore, event: u8, seconds: u32) -> u8 {
    let happening = match event {
        0 => VregDemoEvent::Hang,
        1 => VregDemoEvent::Reboot,
        2 => VregDemoEvent::PowerCycle,
        3 => VregDemoEvent::AdvanceUptime,
        _ => return 0xFF,
    };
    status(core, |c| c.core_vreg_demo_event(happening, seconds))
}

// ── Control surface rules and the firmware guard ─────────────────────

/// Demo device: make the control surfaces report busy, or stop. Returns a
/// status byte.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_demo_busy(core: *mut FfiCore, on: bool) -> u8 {
    status(core, |c| c.cs_demo_busy(on))
}

/// The rows an editor shows for a draft record of `kind` in `slot`, as JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_describe(
    core: *mut FfiCore,
    kind: *const c_char,
    slot: u32,
    draft: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        match arg(kind, "kind").and_then(|k| Ok((k, arg(draft, "draft")?))) {
            Ok((kind, draft)) => c.cs_describe(kind.to_owned(), slot, draft.to_owned()),
            Err(e) => json_reply::<()>(Err(e)),
        }
    })
}

/// A draft with one row changed (`change`: `{id, value}`), as JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_edit(
    core: *mut FfiCore,
    kind: *const c_char,
    slot: u32,
    draft: *const c_char,
    change: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args = arg(kind, "kind").and_then(|k| {
            let d = arg(draft, "draft")?;
            Ok((k, d, arg(change, "change")?))
        });
        match args {
            Ok((kind, draft, change)) => {
                c.cs_edit(kind.to_owned(), slot, draft.to_owned(), change.to_owned())
            }
            Err(e) => json_reply::<()>(Err(e)),
        }
    })
}

/// A new binding for `slot` driven by `component`, as a JSON draft.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_new_binding(
    core: *mut FfiCore,
    slot: u32,
    component: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.cs_new_binding(slot, component))
}

/// A new record of `kind`, as a JSON draft.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_new_record(
    core: *mut FfiCore,
    kind: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| match arg(kind, "kind") {
        Ok(kind) => c.cs_new_record(kind.to_owned()),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

/// A control surface status byte in words (plain text, empty for 0).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_status_text(
    core: *mut FfiCore,
    code: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.cs_status_text(code))
}

/// Demo device: an IR code arrives for the armed learn. False when nothing
/// was armed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_cs_demo_learn(core: *mut FfiCore, protocol: u8, code: u32) -> bool {
    with_core(core, false, |h| {
        h.core.cs_demo_learn(protocol, code).unwrap_or(false)
    })
}

/// What the open board needs before new firmware, in JSON: `{ok,
/// requiredMv, source}`. Read it before entering the bootloader.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_guard_board(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.firmware_guard_board())))
}

/// The board record for a required voltage the caller kept (0: unknown).
fn board_needing(required_mv: u16) -> FirmwareGuardBoard {
    FirmwareGuardBoard {
        required_mv,
        ..FirmwareGuardBoard::default()
    }
}

/// The firmware guard's verdict on an image for a board that needs
/// `board_required_mv`, with what the user `typed` (null: nothing), in JSON:
/// `{ok, allowed, decision, reason, message, confirmationPhrase, imageMv,
/// requiredMv}`. Flash only when `allowed`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_firmware_guard_check(
    image: *const u8,
    image_len: u32,
    board_required_mv: u16,
    typed: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || {
        let verdict = dspi_core::firmware_guard_check(
            c_bytes(image, image_len),
            board_needing(board_required_mv),
            c_str(typed).unwrap_or_default().to_owned(),
        );
        json_reply(Ok(verdict))
    })
}

/// `dspi_flash_to_drive` behind the firmware guard: nothing reaches the drive
/// unless the guard allows it (`firmware_guard` failure otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_flash_to_drive_guarded(
    image: *const u8,
    image_len: u32,
    drive_path: *const c_char,
    platform: i8,
    board_required_mv: u16,
    typed: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || {
        let written = drive_at(drive_path, platform).and_then(|drive| {
            dspi_core::flash_to_drive_guarded(
                c_bytes(image, image_len),
                drive,
                board_needing(board_required_mv),
                c_str(typed).unwrap_or_default().to_owned(),
            )
        });
        done_reply(written)
    })
}

// ── Parameters by path ───────────────────────────────────────────────

/// What `dspi_write_param` takes.
#[derive(serde::Deserialize)]
struct WriteRequest {
    path: String,
    #[serde(default)]
    indices: Vec<u8>,
    value: ParamValue,
}

/// Write one parameter by its registry path. `request` is JSON: `{"path":
/// "out.gain", "indices": [1], "value": {"float": -4}}` (`indices` may be left
/// out; an action's value is `"trigger"`). The reply is `{ok, value}` with
/// the value read back, or a failure; the write has happened whatever the
/// buffer, so do not call again for a short one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_write_param(
    core: *mut FfiCore,
    request: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let written = arg(request, "request")
            .and_then(|r| {
                serde_json::from_str::<WriteRequest>(r).map_err(|e| DspiError::InvalidArgument {
                    msg: format!("the request is not one: {e}"),
                })
            })
            .and_then(|r| c.write_param(r.path, r.indices, r.value))
            .map(|value| json!({ "value": value }));
        json_reply(written)
    })
}

/// Which optional bytes the device's image carried: bit 0 the I2S clock pin
/// mode, bit 1 the S/PDIF inputs' switches, bit 2 the ADAT input switch, bit
/// 3 the ADAT input clock mode. 0 for a null handle or before any read.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_state_presence(core: *const FfiCore) -> u8 {
    with_core(core, 0, |h| {
        let s = h.core.get_state();
        [
            s.i2s_clock_pin_mode_present,
            s.spdif_inputs_enabled_present,
            s.adat_input_enabled_present,
            s.adat_input_clock_mode_present,
        ]
        .into_iter()
        .zip([0x01u8, 0x02, 0x04, 0x08])
        .filter(|&(present, _)| present)
        .fold(0u8, |bits, (_, bit)| bits | bit)
    })
}

/// Every filter type's display name, in JSON (`value`: `[{code, name}]`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filter_type_names(buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || json_reply(Ok(dspi_core::filter_type_names())))
}

/// One filter type's display name, as plain text (`Type <n>` for a code with
/// none).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_filter_type_name(code: u8, buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || dspi_core::filter_type_name(code))
}

/// Whether enabling `output` would collide on Core 1, and why, in JSON:
/// `{ok, conflict, reason}` (`reason` null when there is none).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core1_conflict_reason(
    core: *mut FfiCore,
    output: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let answer = c
            .core1_conflict_reason(output)
            .map(|reason| json!({ "conflict": reason.is_some(), "reason": reason }));
        json_reply(answer)
    })
}

/// Each output's label, in JSON (`value`: a list).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_output_labels(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.output_labels())))
}

/// The output cards (an output pair with its title), in JSON (`value`: a
/// list).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_output_cards(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.output_cards())))
}

// -- WP P0W --
// ── Undo, redo and the change history ───────────────────────────────

/// Step back through the change history (`DspiCore::undo`): the newest
/// change that can be undone is written back through its setter. The reply
/// is `{ok, entry, skipped, note}`: `entry` the change reversed (`null` when
/// nothing moved), `skipped` the changes stepped over that cannot be undone,
/// each as `dspi_history` gives them, `note` why nothing moved when another
/// writer changed the value since (or `null`); or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_undo(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.undo()))
}

/// Step forward again (`DspiCore::redo`): the change undo most recently
/// reversed is written again. The reply is as `dspi_undo`'s (`skipped`
/// always empty).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_redo(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.redo()))
}

/// Whether `dspi_undo` has a change to reverse. False for a null handle or
/// with no device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_can_undo(core: *const FfiCore) -> bool {
    with_core(core, false, |h| h.core.can_undo())
}

/// Whether `dspi_redo` has a change to apply again. False for a null handle
/// or with no device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_can_redo(core: *const FfiCore) -> bool {
    with_core(core, false, |h| h.core.can_redo())
}

/// The newest `n` changes of the open connection, newest first, in JSON
/// (`value`: a list of `{path, indices, before, after, source, atMs,
/// undoable}`; `before` and `after` are parameter values as
/// `dspi_write_param` takes them, `before` null when unknown; `source` is
/// `local`, `undo` or `redo`; `atMs` milliseconds since the Unix epoch). A
/// reconnect starts a new history. `NOTIFY_HISTORY` says when it moved.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_history(core: *mut FfiCore, n: u32, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.history(n))))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_bug1;
// -- WP P0W --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg11;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg12;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg17;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg23;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg24;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg32;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg34;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg36;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0w;
// -- WP CS1 --
#[cfg(test)]
mod tests_wp_cs1;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_q2b;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd31;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd4;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd5;
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd6;
// -- WP QP16 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qp16;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1 {
    use std::ptr::{null, null_mut};

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    const PHRASE: &str = "I ACCEPT THE CORE VOLTAGE RISK";

    fn stock_image() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2");
        std::fs::read(path).unwrap()
    }

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let needed = f(null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
        serde_json::from_slice(&buf[..needed as usize - 1]).unwrap()
    }

    #[test]
    fn clean1_the_guard_blocks_and_takes_the_phrase() {
        let image = stock_image();
        let check = |mv: u16, typed: *const c_char| {
            reply(|b, l| {
                dspi_firmware_guard_check(image.as_ptr(), image.len() as u32, mv, typed, b, l)
            })
        };
        let clear = check(1150, null());
        assert_eq!(clear["ok"], true, "{clear}");
        assert_eq!(clear["allowed"], true, "{clear}");
        let blocked = check(1200, null());
        assert_eq!(blocked["allowed"], false, "{blocked}");
        assert_eq!(blocked["decision"], "needsConfirmation");
        assert_eq!(blocked["confirmationPhrase"], PHRASE);
        let phrase = CString::new(PHRASE).unwrap();
        let taken = check(1200, phrase.as_ptr());
        assert_eq!(taken["allowed"], true, "{taken}");
        assert_eq!(taken["decision"], "confirmed");
        let junk = [0u8; 64];
        let refused =
            reply(|b, l| dspi_firmware_guard_check(junk.as_ptr(), 64, 1150, null(), b, l));
        assert_eq!(refused["allowed"], false);
        assert_eq!(refused["decision"], "refused");
    }

    #[test]
    fn clean1_the_board_is_read_through_c() {
        assert_eq!(dspi_firmware_guard_board(null_mut(), null_mut(), 0), 0);
        let h = dspi_core_new();
        let board = reply(|b, l| dspi_firmware_guard_board(h, b, l));
        assert_eq!(board["ok"], true);
        assert_eq!(board["source"], "noDevice");
        assert_eq!(board["requiredMv"], 0);
        assert!(dspi_open_demo(h));
        let board = reply(|b, l| dspi_firmware_guard_board(h, b, l));
        assert_ne!(board["source"], "noDevice", "{board}");
        dspi_core_free(h);
    }

    #[test]
    fn clean1_a_guarded_flash_stops_before_the_drive() {
        let image = stock_image();
        let path = CString::new("/nonexistent/clean1-drive").unwrap();
        let flash = |typed: *const c_char| {
            reply(|b, l| {
                dspi_flash_to_drive_guarded(
                    image.as_ptr(),
                    image.len() as u32,
                    path.as_ptr(),
                    1,
                    1200,
                    typed,
                    b,
                    l,
                )
            })
        };
        let blocked = flash(null());
        assert_eq!(blocked["ok"], false);
        assert_eq!(blocked["code"], "firmware_guard", "{blocked}");
        // With the phrase the guard lets it through, and the drive refuses.
        let phrase = CString::new(PHRASE).unwrap();
        let tried = flash(phrase.as_ptr());
        assert_eq!(tried["ok"], false);
        assert_ne!(tried["code"], "firmware_guard", "{tried}");
        let no_path = reply(|b, l| {
            dspi_flash_to_drive_guarded(image.as_ptr(), 4, null(), 1, 1150, null(), b, l)
        });
        assert_eq!(no_path["code"], "invalid");
    }

    #[test]
    fn clean1_no_guard_record_has_a_field_named_ok() {
        for record in [
            serde_json::to_value(dspi_core::FirmwareGuardBoard::default()).unwrap(),
            serde_json::to_value(dspi_core::FirmwareGuardVerdict::default()).unwrap(),
            serde_json::to_value(dspi_core::VregCalView::default()).unwrap(),
        ] {
            let fields = record.as_object().unwrap();
            assert!(!fields.contains_key("ok"), "{record}");
        }
    }

    #[test]
    fn clean1_the_link_kind_is_plain_here() {
        let e = DspiError::Link {
            code: "wrong_pin".into(),
            msg: "Wrong PIN".into(),
        };
        let v: Value = serde_json::from_str(&json_reply::<()>(Err(e))).unwrap();
        assert_eq!(v["code"], "link");
        assert_eq!(done_reply(Ok(())), r#"{"ok":true}"#);
    }

    #[test]
    fn clean1_a_document_or_its_failure() {
        assert_eq!(document_reply(Ok("doc".into())), "doc");
        let v: Value = serde_json::from_str(&document_reply(Err(DspiError::ReadOnly))).unwrap();
        assert_eq!(v["code"], "read_only");
    }

    #[test]
    fn clean1_optional_text_refuses_bad_bytes_only() {
        assert_eq!(optional_text(null(), "w"), Ok(None));
        let bad = [0xC3u8, 0];
        assert!(optional_text(bad.as_ptr().cast(), "w").is_err());
        assert_eq!(
            optional_text(c"OVERVOLT".as_ptr(), "w"),
            Ok(Some("OVERVOLT".into()))
        );
    }

    #[test]
    fn clean1_write_param_wants_a_value() {
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let req = CString::new(r#"{"path":"vol.user"}"#).unwrap();
        let r = reply(|b, l| dspi_write_param(h, req.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        dspi_core_free(h);
    }

    #[test]
    fn clean1_the_monitor_needs_a_handle() {
        let mut buf = [0x55u8; 8];
        assert_eq!(
            dspi_monitor_read(null_mut(), 9, buf.as_mut_ptr().cast(), 8),
            9
        );
        assert_eq!(buf, [0x55; 8]);
        let h = dspi_core_new();
        assert_eq!(dspi_monitor_read(h, 0, buf.as_mut_ptr().cast(), 8), 0);
        assert_eq!(buf[0], 0, "an empty read still terminates the buffer");
        dspi_core_free(h);
    }

    #[test]
    fn clean1_wake_and_hotplug_wrappers_unset_on_null() {
        assert!(wake(None, null_mut()).is_none());
        let h = dspi_core_new();
        assert!(dspi_set_hotplug_callback(h, None, null_mut()));
        dspi_set_notify_callback(h, None, null_mut());
        dspi_rta_set_callback(h, None, null_mut());
        dspi_core_free(h);
        assert!(!dspi_set_hotplug_callback(null_mut(), None, null_mut()));
    }

    #[test]
    fn clean1_a_null_chain_of_bytes_is_empty() {
        assert!(c_bytes(null(), 99).is_empty());
        let three = [1u8, 2, 3];
        assert_eq!(c_bytes(three.as_ptr(), 2), vec![1, 2]);
    }
}
