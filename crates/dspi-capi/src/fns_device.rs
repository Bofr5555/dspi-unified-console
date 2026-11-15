//! C functions for the device itself: the handle, finding and opening a
//! DSPi, the mirror, every parameter setter, presets and flash, and the
//! response curves. Each one wraps one [`DspiCore`] method.
//!
//! The helpers at the top are the boundary rules every exported function here
//! and in `fns_misc` follows (crate docs): a null handle, a null or misaligned
//! pointer and a string that is not UTF-8 give the function's failure value;
//! records are copied into the caller's memory; failures are logged, a
//! repeated one at most every 30 s.
//!
//! Failure values: `bool` functions return false; status bytes are the
//! device's own (`PRESET_*`, `PIN_CONFIG_*`, `FLASH_*`) when it refused, and
//! 0xFF (`FLASH_ERR_WRITE` for the flash calls) for anything else; text
//! functions return the bytes they need, 0 for a null handle.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char};
use std::panic::Location;
use std::ptr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use dspi_core::records as r;
use dspi_core::{DspiCore, DspiError, Result};

use crate::types::*;
use crate::{FfiCore, guard, with_core};

// ── Failure logging ──────────────────────────────────────────────────

/// How long a logged failure keeps its repeats quiet.
const QUIET_FOR: Duration = Duration::from_secs(30);

/// How many distinct failures the table remembers.
const REMEMBERED: usize = 64;

/// The failures logged lately: message -> (when it was last logged, repeats
/// held back since).
static RECENT_FAILURES: Mutex<BTreeMap<String, (Instant, u32)>> = Mutex::new(BTreeMap::new());

/// Whether `message` should be logged at `now`, and if so how many repeats of
/// it were held back since it last was. A message is logged the first time,
/// then not again until [`QUIET_FOR`] has passed. The table keeps at most
/// [`REMEMBERED`] messages, forgetting the one logged longest ago.
pub(crate) fn warn_now(
    table: &mut BTreeMap<String, (Instant, u32)>,
    message: &str,
    now: Instant,
) -> Option<u32> {
    if let Some((logged, held)) = table.get_mut(message) {
        if now.saturating_duration_since(*logged) < QUIET_FOR {
            *held = held.saturating_add(1);
            return None;
        }
        let repeats = std::mem::take(held);
        *logged = now;
        return Some(repeats);
    }
    while table.len() >= REMEMBERED {
        let stalest = table
            .iter()
            .min_by_key(|(_, (logged, _))| *logged)
            .map(|(key, _)| key.clone());
        match stalest {
            Some(key) => table.remove(&key),
            None => break,
        };
    }
    table.insert(message.to_owned(), (now, 0));
    Some(0)
}

/// Log a failed call made at `at`. No device is the normal state of an app
/// that polls, so that one is a debug line. The others are held back by
/// their wording alone, wherever they come from: a poll of several calls
/// failing the same way is one line, and the place named is the first.
fn log_failure(e: &DspiError, at: &Location<'_>) {
    note_failure(e); // -- WP QP16 --
    let place = format!("{}:{}", at.file(), at.line());
    if *e == DspiError::Disconnected {
        log::debug!("{e} [{place}]");
        return;
    }
    let message = e.to_string();
    let mut table = RECENT_FAILURES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match warn_now(&mut table, &message, Instant::now()) {
        Some(0) => log::warn!("{message} [{place}]"),
        Some(held) => log::warn!("{message} [{place}], {held} more since the last time"),
        None => {}
    }
}

// -- WP QP16 --
std::thread_local! {
    /// The last failure of a call on this thread, until `dspi_take_failure`
    /// takes it.
    static LAST_FAILURE: std::cell::RefCell<Option<DspiError>> =
        const { std::cell::RefCell::new(None) };
}

/// Keep `e` as this thread's last failure.
fn note_failure(e: &DspiError) {
    LAST_FAILURE.with(|last| *last.borrow_mut() = Some(e.clone()));
}

/// This thread's last failure, left in place unless `take`.
pub(crate) fn last_failure(take: bool) -> Option<DspiError> {
    LAST_FAILURE.with(|last| {
        if take {
            last.borrow_mut().take()
        } else {
            last.borrow().clone()
        }
    })
}

/// The value of `result`, logging a failure.
fn settle<T>(result: Result<T>, at: &Location<'_>) -> Option<T> {
    result.map_err(|e| log_failure(&e, at)).ok()
}

// ── Arguments and buffers ────────────────────────────────────────────

/// Borrow a NUL-terminated string argument: `None` for null or for bytes that
/// are not UTF-8.
pub(crate) fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a non-null string argument points at a NUL-terminated string
    // that lives for the call; the C caller's contract.
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str().ok()
}

/// A required string argument, or the error that says which one is wrong.
pub(crate) fn arg<'a>(p: *const c_char, name: &str) -> Result<&'a str> {
    c_str(p).ok_or_else(|| DspiError::InvalidArgument {
        msg: format!("{name} is null or not UTF-8"),
    })
}

/// Whether `p` can be written as one `T`: not null, and aligned.
fn writable<T>(p: *mut T) -> bool {
    !p.is_null() && p.is_aligned()
}

/// Store `value` at `p` if it is [`writable`].
fn put<T>(p: *mut T, value: T) -> bool {
    if !writable(p) {
        return false;
    }
    // SAFETY: `p` is non-null and aligned, and points at a `T` the caller
    // owns; the C caller's contract.
    unsafe { p.write(value) };
    true
}

/// Fill `count` elements from `out` on with `values`, then `fill` for any
/// that `values` does not cover. `out` must be [`writable`].
fn put_array<T: Copy>(out: *mut T, count: usize, values: &[T], fill: T) {
    let padded = values.iter().copied().chain(std::iter::repeat(fill));
    for (i, value) in padded.take(count).enumerate() {
        // SAFETY: the caller's array holds `count` elements (the C caller's
        // contract, documented on each function) and `i < count`.
        unsafe { out.add(i).write(value) };
    }
}

/// Copy as much of `s` as fits into `buf` of `len` bytes, cut at a character
/// boundary and NUL-terminated. False, with nothing written, for a null
/// buffer or a length of 0.
pub(crate) fn copy_out(s: &str, buf: *mut c_char, len: u32) -> bool {
    if buf.is_null() || len == 0 {
        return false;
    }
    let room = usize::try_from(len).map_or(usize::MAX, |n| n.saturating_sub(1));
    let mut end = s.len().min(room);
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let head = s.as_bytes().get(..end).unwrap_or_default();
    // SAFETY: `buf` holds `len` bytes (the C caller's contract); `head` and
    // its NUL take at most `len` of them, and a Rust string cannot overlap
    // the caller's buffer.
    unsafe {
        ptr::copy_nonoverlapping(head.as_ptr(), buf.cast::<u8>(), head.len());
        buf.add(head.len()).write(0);
    }
    true
}

/// Copy all of `s` into `buf` with its NUL, or nothing when it does not fit
/// in `len` bytes. Returns the bytes needed, NUL included, either way: the
/// caller can ask with a null buffer and call again.
pub(crate) fn copy_text(s: &str, buf: *mut c_char, len: u32) -> u32 {
    let needed = s
        .len()
        .checked_add(1)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(u32::MAX);
    if buf.is_null() || needed > len {
        return needed;
    }
    // SAFETY: `buf` holds `len` bytes (the C caller's contract), which is room
    // for `s` and its NUL, and a Rust string cannot overlap it.
    unsafe {
        ptr::copy_nonoverlapping(s.as_ptr(), buf.cast::<u8>(), s.len());
        buf.add(s.len()).write(0);
    }
    needed
}

// ── Call shapes ──────────────────────────────────────────────────────

/// The status byte for `result`: 0, the device's own byte when it refused,
/// `fail` for every other error.
pub(crate) fn status_of(result: Result<()>, fail: u8) -> u8 {
    match result {
        Ok(()) => 0,
        Err(DspiError::Refused { status }) => status,
        Err(_) => fail,
    }
}

/// A call answered with a status byte; 0xFF for a null handle, a panic or an
/// error other than the device's refusal.
#[track_caller]
pub(crate) fn status(core: *mut FfiCore, f: impl FnOnce(&DspiCore) -> Result<()>) -> u8 {
    status_or(core, 0xFF, f)
}

/// [`status`] with another failure value.
#[track_caller]
fn status_or(core: *mut FfiCore, fail: u8, f: impl FnOnce(&DspiCore) -> Result<()>) -> u8 {
    let at = Location::caller();
    with_core(core, fail, |h| {
        let result = f(&h.core);
        if let Err(e) = &result {
            log_failure(e, at);
        }
        status_of(result, fail)
    })
}

/// A call answered true or false: false for a null handle, a panic or an
/// error.
#[track_caller]
pub(crate) fn flag(core: *mut FfiCore, f: impl FnOnce(&DspiCore) -> Result<()>) -> bool {
    let at = Location::caller();
    with_core(core, false, |h| settle(f(&h.core), at).is_some())
}

/// A call whose answer is a value: `fallback` for a null handle, a panic or
/// an error.
#[track_caller]
fn value_or<T>(core: *const FfiCore, fallback: T, f: impl FnOnce(&DspiCore) -> Result<T>) -> T {
    let at = Location::caller();
    with_core(core, None, |h| settle(f(&h.core), at)).unwrap_or(fallback)
}

/// A call that fills one record at `out`. False, with `out` untouched, for a
/// null handle, a null or misaligned `out`, a panic or an error; `f` is not
/// called when `out` is unusable.
#[track_caller]
pub(crate) fn fetch_value<T>(
    core: *mut FfiCore,
    out: *mut T,
    f: impl FnOnce(&DspiCore) -> Result<T>,
) -> bool {
    let at = Location::caller();
    if !writable(out) {
        return false;
    }
    with_core(core, false, |h| {
        settle(f(&h.core), at).is_some_and(|value| put(out, value))
    })
}

/// A call answered with text: the bytes needed (see [`copy_text`]), 0 for a
/// null handle or a panic.
pub(crate) fn text(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
    f: impl FnOnce(&DspiCore) -> String,
) -> u32 {
    with_core(core, 0, |h| {
        // -- WP ROMODE -- a refusal of the client read-only mode has its own row
        let reply = crate::fns_misc::read_only::refine_reply(&h.core, f(&h.core));
        copy_text(&reply, buf, len)
    })
}

/// A name read into `out` of `len` bytes, cut to fit ([`copy_out`]). False
/// for an unusable buffer, a null handle, a panic or an error.
#[track_caller]
pub(crate) fn name_into(
    core: *mut FfiCore,
    out: *mut c_char,
    len: u32,
    f: impl FnOnce(&DspiCore) -> Result<String>,
) -> bool {
    let at = Location::caller();
    if out.is_null() || len == 0 {
        return false;
    }
    with_core(core, false, |h| {
        settle(f(&h.core), at).is_some_and(|name| copy_out(&name, out, len))
    })
}

/// Per-output values into `out`, which holds `MAX_OUTPUTS` floats: what the
/// device gave, then zeros. False, with `out` untouched, on a failure.
#[track_caller]
pub(crate) fn fetch_floats(
    core: *mut FfiCore,
    out: *mut f32,
    f: impl FnOnce(&DspiCore) -> Result<Vec<f32>>,
) -> bool {
    let at = Location::caller();
    if !writable(out) {
        return false;
    }
    with_core(core, false, |h| match settle(f(&h.core), at) {
        Some(values) => {
            put_array(out, MAX_OUTPUTS, &values, 0.0);
            true
        }
        None => false,
    })
}

/// A curve into `out`, which holds `MAGNITUDE_POINTS` doubles: the values,
/// zeros after a short curve, a long one cut.
pub(crate) fn write_curve(values: Vec<f64>, out: *mut f64) -> bool {
    if !writable(out) {
        return false;
    }
    put_array(out, MAGNITUDE_POINTS, &values, 0.0);
    true
}

/// Up to `max` devices into `out` (none when it is null or misaligned).
/// Returns how many that is.
pub(crate) fn write_devices(devices: &[r::DeviceInfo], out: *mut DeviceInfo, max: u32) -> u32 {
    let room = usize::try_from(max).unwrap_or(usize::MAX);
    let count = devices.len().min(room);
    if writable(out) {
        let records: Vec<DeviceInfo> = devices.iter().take(count).map(DeviceInfo::from).collect();
        put_array(out, count, &records, DeviceInfo::default());
    }
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The filter chain a caller passes: `count` records at `filters`. Null (or
/// misaligned) is an empty chain.
fn chain(filters: *const FilterParams, count: u32) -> Vec<r::FilterParams> {
    if filters.is_null() || !filters.is_aligned() {
        return Vec::new();
    }
    let count = usize::try_from(count).unwrap_or(0);
    // SAFETY: a non-null chain holds `count` records; the C caller's contract.
    let bands = unsafe { std::slice::from_raw_parts(filters, count) };
    bands.iter().map(r::FilterParams::from).collect()
}

// ── The handle ───────────────────────────────────────────────────────

/// Create a core handle, with no device open, and install the logger (stderr,
/// level from `RUST_LOG`). Free it with `dspi_core_free`. Null only if
/// creating it panicked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_new() -> *mut FfiCore {
    guard(ptr::null_mut(), || {
        crate::init_logger();
        let core = DspiCore::new();
        let state = Box::new(DspState::from(&core.get_state()));
        let handle = FfiCore {
            core,
            state: Mutex::new(state),
        };
        Box::into_raw(Box::new(handle))
    })
}

/// Free a handle from `dspi_core_new`. Every callback is unset first, so none
/// runs once this has begun; then the Link session ends (in a build with
/// Link) and the device is closed. The `DspState` pointer from
/// `dspi_get_state` dies with the handle. Null is ignored.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_core_free(core: *mut FfiCore) {
    if core.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: a non-null handle comes from dspi_core_new and is not used
        // again after this call; the C caller's contract.
        let handle = unsafe { Box::from_raw(core) };
        let c = &handle.core;
        c.set_notify_listener(None);
        c.rta_set_listener(None);
        c.set_hotplug_callback(None);
        #[cfg(feature = "link")]
        {
            c.set_link_listener(None);
            c.link_close();
        }
        #[cfg(feature = "mdns")]
        c.link_browse_stop();
        c.disconnect();
    });
}

// ── Finding and opening a device ─────────────────────────────────────

/// Look for DSPi units on USB and write up to `max_devices` of them to
/// `out_devices` (may be null: then nothing is written). Returns how many
/// were written, or would have been; 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_scan_devices(
    core: *mut FfiCore,
    out_devices: *mut DeviceInfo,
    max_devices: u32,
) -> u32 {
    with_core(core, 0, |h| {
        write_devices(&h.core.scan_devices(), out_devices, max_devices)
    })
}

/// The largest preamp gain the open device takes, in dB (its platform's
/// limit; the default without a device). 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_preamp_max_db(core: *mut FfiCore) -> f32 {
    with_core(core, 0.0, |h| h.core.preamp_max_db())
}

/// How many DSPi units the last scan saw but could not open (permissions,
/// another program). 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_inaccessible_devices(core: *mut FfiCore) -> u32 {
    with_core(core, 0, |h| h.core.inaccessible_devices())
}

/// Open the DSPi with this serial (from `dspi_scan_devices`) and read it.
/// False for a null or non-UTF-8 serial, or when it cannot be opened.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_select_device(core: *mut FfiCore, serial: *const c_char) -> bool {
    flag(core, |c| c.select_device(arg(serial, "serial")?.to_owned()))
}

/// Close the open device, if any; the mirror goes back to its defaults.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_disconnect(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.disconnect());
}

/// Whether a device is open. False for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_is_connected(core: *const FfiCore) -> bool {
    with_core(core, false, |h| h.core.is_connected())
}

// ── The mirror ───────────────────────────────────────────────────────

/// Read everything from the open device into the mirror.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_all(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::fetch_all)
}

/// Read the parameters again (the bulk image), not the platform facts.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_refresh_params(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::refresh_params)
}

/// Write every parameter of the mirror to the device.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_apply_all_params(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::apply_all_params)
}

/// Read the live status (levels, clips, load). `out_status` may be null: the
/// read still happens. False on failure or for a misaligned `out_status`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_status(core: *mut FfiCore, out_status: *mut SystemStatus) -> bool {
    if !out_status.is_null() && !out_status.is_aligned() {
        return false;
    }
    let at = Location::caller();
    with_core(core, false, |h| match settle(h.core.fetch_status(), at) {
        Some(status) => {
            if !out_status.is_null() {
                put(out_status, SystemStatus::from(&status));
            }
            true
        }
        None => false,
    })
}

/// The mirror, refreshed now. The record belongs to the handle and stays at
/// the same address until `dspi_core_free`; each call rewrites it, so read it
/// from the thread that calls this. Null for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_state(core: *const FfiCore) -> *const DspState {
    with_core(core, ptr::null(), |h| {
        let fresh = DspState::from(&h.core.get_state());
        let mut held = h.state.lock().unwrap_or_else(PoisonError::into_inner);
        **held = fresh;
        let record: &DspState = &held;
        ptr::from_ref(record)
    })
}

/// The longest delay the open device takes, in ms. 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_max_delay_ms(core: *const FfiCore) -> f32 {
    with_core(core, 0.0, |h| h.core.max_delay_ms())
}

/// Clear the device's clip indicators.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_clear_clips(core: *mut FfiCore) -> bool {
    flag(core, DspiCore::clear_clips)
}

// ── Filters and levels ───────────────────────────────────────────────

/// Set PEQ band `band` of wire channel `ch`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_filter(
    core: *mut FfiCore,
    ch: u8,
    band: u8,
    params: FilterParams,
) -> bool {
    flag(core, |c| {
        c.set_filter(ch, band, r::FilterParams::from(&params))
    })
}

/// Set crossover band `xband` of wire channel `ch`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossover(
    core: *mut FfiCore,
    ch: u8,
    xband: u8,
    params: FilterParams,
) -> bool {
    flag(core, |c| {
        c.set_crossover(ch, xband, r::FilterParams::from(&params))
    })
}

/// Bypass, or stop bypassing, one band.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_band_bypass(core: *mut FfiCore, ch: u8, band: u8, bypass: bool) -> bool {
    flag(core, |c| c.set_band_bypass(ch, band, bypass))
}

/// Set the global preamp, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_preamp(core: *mut FfiCore, db: f32) -> bool {
    flag(core, |c| c.set_preamp(db))
}

/// Set the preamp of one input, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_preamp(core: *mut FfiCore, input: u8, db: f32) -> bool {
    flag(core, |c| c.set_input_preamp(input, db))
}

// -- WP P1PR --
/// Set the preamp of a linked input pair (`input` and `input ^ 1`), in dB:
/// one undo step (`DspiCore::set_input_pair_preamp`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_pair_preamp(core: *mut FfiCore, input: u8, db: f32) -> bool {
    flag(core, |c| c.set_input_pair_preamp(input, db))
}

// -- WP PAIRBAND --
/// Set PEQ band `band` of a linked input pair (`input` and `input ^ 1`, their
/// wire channels): one undo step (`DspiCore::set_input_pair_filter`). Call it
/// for every band edit of a linked pair, a drag's ticks included.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_pair_filter(
    core: *mut FfiCore,
    input: u8,
    band: u8,
    params: FilterParams,
) -> bool {
    flag(core, |c| {
        c.set_input_pair_filter(input, band, r::FilterParams::from(&params))
    })
}

// -- WP PAIRBAND --
/// Bypass or re-enable PEQ band `band` of a linked input pair: one undo step
/// (`DspiCore::set_input_pair_band_bypass`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_pair_band_bypass(
    core: *mut FfiCore,
    input: u8,
    band: u8,
    bypass: bool,
) -> bool {
    flag(core, |c| c.set_input_pair_band_bypass(input, band, bypass))
}

/// Bypass all processing, or stop.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_bypass(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_bypass(enabled))
}

/// Set the master volume, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_master_volume(core: *mut FfiCore, db: f32) -> bool {
    flag(core, |c| c.set_master_volume(db))
}

/// Set how the master volume is kept (with the preset, or on its own).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_master_volume_mode(core: *mut FfiCore, mode: u8) -> bool {
    flag(core, |c| c.set_master_volume_mode(mode))
}

/// Store the master volume on the device. Returns a status byte: 0, the
/// device's refusal, or 0xFF.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_master_volume(core: *mut FfiCore) -> u8 {
    status(core, DspiCore::save_master_volume)
}

/// Set the user volume, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_user_volume(core: *mut FfiCore, db: f32) -> bool {
    flag(core, |c| c.set_user_volume(db))
}

/// Mute or unmute the user volume.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_user_mute(core: *mut FfiCore, muted: bool) -> bool {
    flag(core, |c| c.set_user_mute(muted))
}

/// Set the delay of wire channel `ch`, in ms.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_delay(core: *mut FfiCore, ch: u8, ms: f32) -> bool {
    flag(core, |c| c.set_delay(ch, ms))
}

// ── Effects ──────────────────────────────────────────────────────────

/// Turn loudness compensation on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_loudness(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_loudness(enabled))
}

/// Set the loudness reference level, in dB SPL.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_loudness_ref(core: *mut FfiCore, spl: f32) -> bool {
    flag(core, |c| c.set_loudness_ref(spl))
}

/// Set the loudness intensity, in percent.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_loudness_intensity(core: *mut FfiCore, pct: f32) -> bool {
    flag(core, |c| c.set_loudness_intensity(pct))
}

/// Set which channels get loudness compensation.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_loudness_mask(core: *mut FfiCore, mask: u16) -> bool {
    flag(core, |c| c.set_loudness_mask(mask))
}

/// Turn the crossfeed on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_crossfeed(enabled))
}

/// Choose a crossfeed preset.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_preset(core: *mut FfiCore, preset: u8) -> bool {
    flag(core, |c| c.set_crossfeed_preset(preset))
}

/// Set the custom crossfeed frequency, in Hz.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_freq(core: *mut FfiCore, freq: f32) -> bool {
    flag(core, |c| c.set_crossfeed_freq(freq))
}

/// Set the custom crossfeed level, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_feed(core: *mut FfiCore, feed: f32) -> bool {
    flag(core, |c| c.set_crossfeed_feed(feed))
}

// -- WP P1AFIX --
/// The crossfeed's custom pair as the device keeps it, whatever the preset
/// (`dspi_get_state` shows a named preset's values instead), into `out`.
/// False with no device. No transfer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_crossfeed_custom_pair(
    core: *mut FfiCore,
    out: *mut CrossfeedPair,
) -> bool {
    fetch_value(core, out, |c| {
        c.crossfeed_custom_pair().map(CrossfeedPair::from)
    })
}

/// Turn the crossfeed's interaural time delay on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_itd(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_crossfeed_itd(enabled))
}

/// Set which output pairs the crossfeed processes.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_crossfeed_outputs(core: *mut FfiCore, pair_mask: u8) -> bool {
    flag(core, |c| c.set_crossfeed_outputs(pair_mask))
}

/// Turn the leveller on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_enabled(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_leveller_enabled(enabled))
}

/// Set the leveller's amount, in percent.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_amount(core: *mut FfiCore, pct: f32) -> bool {
    flag(core, |c| c.set_leveller_amount(pct))
}

/// Set the leveller's speed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_speed(core: *mut FfiCore, speed: u8) -> bool {
    flag(core, |c| c.set_leveller_speed(speed))
}

/// Set the leveller's largest gain, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_max_gain(core: *mut FfiCore, db: f32) -> bool {
    flag(core, |c| c.set_leveller_max_gain(db))
}

/// Turn the leveller's lookahead on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_lookahead(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_leveller_lookahead(enabled))
}

/// Set the leveller's gate, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_gate(core: *mut FfiCore, db: f32) -> bool {
    flag(core, |c| c.set_leveller_gate(db))
}

/// Set which channels the leveller listens to and which it acts on.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_leveller_masks(core: *mut FfiCore, detector: u8, apply: u8) -> bool {
    flag(core, |c| c.set_leveller_masks(detector, apply))
}

/// Choose the input source.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_input_source(core: *mut FfiCore, source: u8) -> bool {
    flag(core, |c| c.set_input_source(source))
}

/// Set one psychoacoustic bass parameter.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_psybass_param(core: *mut FfiCore, param: u8, value: f32) -> bool {
    flag(core, |c| c.set_psybass_param(param, value))
}

/// Set which outputs get psychoacoustic bass.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_psybass_mask(core: *mut FfiCore, mask: u16) -> bool {
    flag(core, |c| c.set_psybass_mask(mask))
}

/// Turn psychoacoustic bass on or off.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_psybass_enabled(core: *mut FfiCore, enabled: bool) -> bool {
    flag(core, |c| c.set_psybass_enabled(enabled))
}

// ── Matrix and outputs ───────────────────────────────────────────────

/// Set one crosspoint of the matrix: input to output, on or off, its gain in
/// dB and its polarity.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_matrix_route(
    core: *mut FfiCore,
    input: u8,
    output: u8,
    enabled: bool,
    gain: f32,
    invert: bool,
) -> bool {
    flag(core, |c| {
        c.set_matrix_route(input, output, enabled, gain, invert)
    })
}

/// Enable or disable an output. Returns the state it ended in, read back: 1
/// enabled, 0 disabled (an enable that would collide on Core 1 is refused,
/// and gives 0), -1 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_enable(core: *mut FfiCore, output: u8, enabled: bool) -> i8 {
    value_or(core, -1, |c| {
        c.set_output_enable(output, enabled).map(i8::from)
    })
}

/// Set an output's gain, in dB.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_gain(core: *mut FfiCore, output: u8, db: f32) -> bool {
    flag(core, |c| c.set_output_gain(output, db))
}

/// Mute or unmute an output.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_mute(core: *mut FfiCore, output: u8, muted: bool) -> bool {
    flag(core, |c| c.set_output_mute(output, muted))
}

/// Set an output's delay, in ms.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_output_delay(core: *mut FfiCore, output: u8, ms: f32) -> bool {
    flag(core, |c| c.set_output_delay(output, ms))
}

/// Set one limiter parameter of an output.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_limiter_param(
    core: *mut FfiCore,
    output: u8,
    param: u8,
    value: f32,
) -> bool {
    flag(core, |c| c.set_limiter_param(output, param, value))
}

/// The limiters' gain reduction, in dB, into `out_gr`: `MAX_OUTPUTS` floats.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_limiter_meter(core: *mut FfiCore, out_gr: *mut f32) -> bool {
    fetch_floats(core, out_gr, DspiCore::fetch_limiter_meter)
}

// -- WP P1PR --
/// Protect every output: stereo pairs linked, every limiter on at
/// `ceiling_dbfs` (-1 is the usual ceiling), one undo step
/// (`DspiCore::protect_all_outputs`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_protect_all_outputs(core: *mut FfiCore, ceiling_dbfs: f32) -> bool {
    flag(core, |c| c.protect_all_outputs(ceiling_dbfs))
}

// -- WP P1PR --
/// The outputs that are on, above `threshold_db` of gain and without a
/// limiter, as a mask (bit k = output k; `DspiCore::outputs_without_limiter`,
/// pass -10 for the warning). 0 for a null handle or none.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_outputs_without_limiter(core: *const FfiCore, threshold_db: f32) -> u32 {
    value_or(core, 0, |c| {
        Ok(c.outputs_without_limiter(threshold_db)
            .into_iter()
            .fold(0u32, |m, o| m | 1u32.checked_shl(u32::from(o)).unwrap_or(0)))
    })
}

/// Name wire channel `ch`. False for a null or non-UTF-8 name, or one the
/// device does not take.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_channel_name(core: *mut FfiCore, ch: u8, name: *const c_char) -> bool {
    flag(core, |c| {
        c.set_channel_name(ch, arg(name, "name")?.to_owned())
    })
}

/// The name of wire channel `ch` into `out_buf` of `buf_len` bytes, cut to fit
/// and NUL-terminated.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_channel_name(
    core: *mut FfiCore,
    ch: u8,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, |c| c.get_channel_name(ch))
}

// ── Upmixer, sub harmonics, tube ─────────────────────────────────────

/// Set one upmixer parameter.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_upmix_param(core: *mut FfiCore, id: u8, value: f32) -> bool {
    flag(core, |c| c.set_upmix_param(id, value))
}

/// Read the upmixer's live status into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_upmix_status(core: *mut FfiCore, out: *mut UpmixStatus) -> bool {
    fetch_value(core, out, |c| {
        c.fetch_upmix_status().map(|s| UpmixStatus::from(&s))
    })
}

/// Set one sub-harmonic synthesiser parameter.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_subharm_param(core: *mut FfiCore, id: u8, value: f32) -> bool {
    flag(core, |c| c.set_subharm_param(id, value))
}

/// Solo the sub-harmonic synthesiser's output, or stop.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_subharm_solo(core: *mut FfiCore, solo: bool) -> bool {
    flag(core, |c| c.set_subharm_solo(solo))
}

/// The sub-harmonic synthesiser's headroom, in dB, into `out_db`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_subharm_headroom(core: *mut FfiCore, out_db: *mut f32) -> bool {
    fetch_value(core, out_db, DspiCore::fetch_subharm_headroom)
}

/// The sub-harmonic synthesiser's per-output meter into `out`:
/// `MAX_OUTPUTS` floats.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_fetch_subharm_meter(core: *mut FfiCore, out: *mut f32) -> bool {
    fetch_floats(core, out, DspiCore::fetch_subharm_meter)
}

/// Set one tube stage parameter.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_tube_param(core: *mut FfiCore, idx: u8, value: f32) -> bool {
    flag(core, |c| c.set_tube_param(idx, value))
}

// ── Presets and flash ────────────────────────────────────────────────

/// Load preset `slot` and read the device again. Returns a `PRESET_*`
/// status, 0xFF when it could not be asked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_load_preset(core: *mut FfiCore, slot: u8) -> u8 {
    status(core, |c| c.load_preset(slot))
}

/// Name preset `slot`. False for a null or non-UTF-8 name.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_preset_name(core: *mut FfiCore, slot: u8, name: *const c_char) -> bool {
    flag(core, |c| {
        c.set_preset_name(slot, arg(name, "name")?.to_owned())
    })
}

/// The name of preset `slot` into `out_buf` of `buf_len` bytes, cut to fit
/// and NUL-terminated.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_preset_name(
    core: *mut FfiCore,
    slot: u8,
    out_buf: *mut c_char,
    buf_len: u32,
) -> bool {
    name_into(core, out_buf, buf_len, |c| c.get_preset_name(slot))
}

/// Read the preset directory into `out_dir`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_get_preset_directory(
    core: *mut FfiCore,
    out_dir: *mut PresetDirectory,
) -> bool {
    fetch_value(core, out_dir, |c| {
        c.get_preset_directory().map(|d| PresetDirectory::from(&d))
    })
}

/// Choose what the device loads at power-up: `mode`, and the slot for the
/// mode that names one.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_set_preset_startup(core: *mut FfiCore, mode: u8, default_slot: u8) -> bool {
    flag(core, |c| c.set_preset_startup(mode, default_slot))
}

/// Load the parameters saved in flash. Returns a `FLASH_*` status;
/// `FLASH_ERR_WRITE` when it could not be asked.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_load_params(core: *mut FfiCore) -> u8 {
    status_or(core, FLASH_ERR_WRITE, DspiCore::load_params)
}

/// Take the live state as the one unsaved changes are counted from.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_capture_baseline(core: *mut FfiCore) {
    with_core(core, (), |h| h.core.capture_baseline());
}

/// Whether the live state differs from the baseline. False for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_preset_dirty(core: *mut FfiCore) -> bool {
    with_core(core, false, |h| h.core.preset_dirty())
}

/// What differs from the baseline, as a JSON array of sentences, into `buf`
/// of `len` bytes. Returns the bytes needed including the NUL; nothing is
/// written when that is more than `len`. 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_preset_changes(core: *mut FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| {
        serde_json::Value::from(c.preset_changes()).to_string()
    })
}

// ── Curves ───────────────────────────────────────────────────────────

/// The response of `num_filters` bands at `filters` at `freq` Hz, in dB. A
/// null or empty chain is flat: 0.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_compute_response(
    filters: *const FilterParams,
    num_filters: u32,
    freq: f32,
) -> f32 {
    guard(0.0, || {
        let bands = chain(filters, num_filters);
        if bands.is_empty() {
            0.0
        } else {
            dspi_core::compute_response(bands, freq)
        }
    })
}

/// The magnitude curve of a chain, in dB, into `out_magnitudes`:
/// `MAGNITUDE_POINTS` doubles on the core's frequency grid. A null chain is
/// flat. False for a null `out_magnitudes`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_compute_magnitude_curve(
    filters: *const FilterParams,
    num_filters: u32,
    out_magnitudes: *mut f64,
) -> bool {
    if !writable(out_magnitudes) {
        return false;
    }
    guard(false, || {
        let curve = dspi_core::compute_magnitude_curve(chain(filters, num_filters));
        write_curve(curve, out_magnitudes)
    })
}

/// The phase curve of a chain, in degrees, unwrapped if asked, into
/// `out_phases`: `MAGNITUDE_POINTS` doubles. False for a null `out_phases`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_compute_phase_curve(
    filters: *const FilterParams,
    num_filters: u32,
    unwrap: bool,
    out_phases: *mut f64,
) -> bool {
    if !writable(out_phases) {
        return false;
    }
    guard(false, || {
        let curve = dspi_core::compute_phase_curve(chain(filters, num_filters), unwrap);
        write_curve(curve, out_phases)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_q2a;

// -- WP P1PR --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1pr;

// -- WP PAIRBAND --
#[cfg(test)]
mod tests_wp_pairband;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1 {
    use super::*;

    #[test]
    fn clean1_put_refuses_null_and_misaligned() {
        let mut words = [0u32; 2];
        assert!(!put(ptr::null_mut::<u32>(), 5));
        let odd = words
            .as_mut_ptr()
            .cast::<u8>()
            .wrapping_add(1)
            .cast::<u32>();
        assert!(!put(odd, 5));
        assert!(put(words.as_mut_ptr(), 5));
        assert_eq!(words, [5, 0]);
    }

    #[test]
    fn clean1_a_status_keeps_the_device_byte_and_maps_the_rest() {
        assert_eq!(status_of(Err(DspiError::Locked), 7), 7);
        assert_eq!(status_of(Err(DspiError::Refused { status: 0x04 }), 7), 4);
        assert_eq!(status(ptr::null_mut(), |_| Ok(())), 0xFF);
        assert!(!flag(ptr::null_mut(), |_| Ok(())));
    }

    #[test]
    fn clean1_a_failed_fetch_leaves_the_record_alone() {
        let h = dspi_core_new();
        let mut v = 3.5f32;
        assert!(!fetch_value(h, &raw mut v, |_| Err(DspiError::Unsupported)));
        assert_eq!(v, 3.5);
        let mut never = false;
        assert!(!fetch_value(h, ptr::null_mut::<f32>(), |_| {
            never = true;
            Ok(1.0)
        }));
        assert!(!never, "nothing is asked for a null out");
        dspi_core_free(h);
    }

    #[test]
    fn clean1_names_cut_to_the_buffer() {
        let mut buf = [0x55 as c_char; 4];
        assert!(!name_into(ptr::null_mut(), buf.as_mut_ptr(), 4, |_| Ok(
            "x".into()
        )));
        let h = dspi_core_new();
        assert!(name_into(h, buf.as_mut_ptr(), 4, |_| Ok("abcdef".into())));
        // SAFETY: copy_out terminated it.
        assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }, c"abc");
        assert!(!name_into(h, buf.as_mut_ptr(), 0, |_| Ok("x".into())));
        dspi_core_free(h);
    }

    #[test]
    fn clean1_free_runs_on_a_handle_with_a_device() {
        let h = dspi_core_new();
        with_core(h, (), |c| c.core.open_demo().unwrap());
        assert!(dspi_is_connected(h));
        dspi_core_free(h);
    }

    #[test]
    fn clean1_the_failure_log_holds_back_repeats_per_message() {
        let mut t = BTreeMap::new();
        let t0 = Instant::now();
        assert_eq!(warn_now(&mut t, "x", t0), Some(0));
        for s in 1..30 {
            assert_eq!(warn_now(&mut t, "x", t0 + Duration::from_secs(s)), None);
        }
        assert_eq!(
            warn_now(&mut t, "x", t0 + Duration::from_secs(30)),
            Some(29)
        );
    }
}
