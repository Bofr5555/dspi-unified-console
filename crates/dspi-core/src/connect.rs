//! Reading the device into the mirror, and writing the mirror back.
//!
//! - [`DspiCore::fetch_all`] reads the firmware identity, the bulk image in
//!   whatever wire format the firmware speaks (`dspi_session::compat`,
//!   mapped by `dspi_proto::wire_compat`), decodes it with the mirror codec
//!   ([`crate::state`]) and then the values the image does not carry
//!   (preset directory and names, Core 1 mode, subharm solo, DAC mute
//!   support, control interfaces).
//! - An image in another wire format makes the connection **read-only**
//!   (PLAN §1.5): `Connection::wire` is `Older`/`Newer` (or `None` when the
//!   image cannot be mapped at all), `compat` says why, `bulk_valid` is
//!   false, and the image is never written back.
//! - [`DspiCore::apply_all_params`] writes the mirror back in one bulk
//!   transfer (chunked `0xA3`, or single-shot `0xA1` where the link has no
//!   chunked bulk, as DSPi Link), bracketed with the connection's
//!   [`dspi_session::notify::EchoFilter`] so its `BULK_INVALIDATED` is
//!   recognised as our own (PLAN §1.2).
//!
//! The per-item readers (`read_*`) are `pub(crate)` so the WPs that own the
//! public `fetch_*` / `get_preset_*` methods can reuse them; they work on a
//! [`Connection`] and a [`DspState`] and never take the lock.
//!
//! Owned by WP C2 (docs/core-merge/PLAN.md).

use std::sync::Arc;

use dspi_transport::clock::Instant;

use dspi_proto::generated::opcodes as op;
use dspi_proto::wire::WireError;
use dspi_proto::wire_compat::{self, WireRead};
use dspi_session::WriteError;
use dspi_transport::{InRead, TransportError};

use crate::consts::*;
use crate::records::*;
use crate::{Connection, DspiCore, DspiError, Inner, Result, Writable, presets, state};

/// The wire format this build reads and writes.
const V32: u8 = dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8;

/// USB vendor id of the pre-May-2026 firmware (Raspberry Pi's): its wire
/// format is far older than anything this build maps, so a failed bulk read
/// there means "too old", not "unreachable".
const LEGACY_VID: u16 = 0x2E8A;

/// The largest `0xA3` chunk sent in one transfer, as the old core did; a
/// transport with a smaller `max_transfer` gets smaller chunks.
const BULK_WRITE_CHUNK: usize = 2048;

/// Why `apply_all_params` takes the Link device lock (shown to other
/// consoles).
pub(crate) const LOCK_REASON: &str = "Applying configuration";

/// `REQ_GET_PLATFORM` answers 4, 6 or 7 bytes, a DSPi-Zero 8 (byte 7 the board
/// id). -- WP PGFWUP -- asked for `PlatformInfo::FULL_LEN` (PLAN-PG section 1.9).
const PLATFORM_LEN: u16 = {
    assert!(dspi_proto::packets::PlatformInfo::FULL_LEN <= u16::MAX as usize);
    dspi_proto::packets::PlatformInfo::FULL_LEN as u16
};

/// Bytes of `REQ_GET_DAC_HW_MUTE_CONFIG` used: enabled, active-low, pin,
/// pad, hold ms, release ms (the first 8 bytes of the image's section).
const DAC_MUTE_REPLY: usize = 8;

/// `REQ_GET_STATUS` sub-query: the meter block.
const STATUS_METERS: u16 = 9;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Identify the firmware and read every parameter into the mirror.
    ///
    /// Reads `REQ_GET_PLATFORM`, then the bulk image in whatever wire format
    /// the firmware speaks (chunked `0xA2`, or single-shot `0xA0` over a link
    /// without chunked bulk such as DSPi Link), then the values the image
    /// does not carry. The mirror is rebuilt from defaults, so nothing of a
    /// previous device survives; it is replaced only when the call succeeds.
    ///
    /// Sets `DspState.compat`:
    /// - `COMPAT_OK`: a V32 image, readable and writable;
    /// - `COMPAT_FIRMWARE_TOO_OLD` / `COMPAT_FIRMWARE_TOO_NEW`: another wire
    ///   format. When it can be mapped (V16..V31, or a newer image's V32
    ///   prefix) the mirror shows it; either way the connection is
    ///   read-only: `bulk_valid` is false and every setter (and
    ///   [`DspiCore::apply_all_params`]) returns [`DspiError::ReadOnly`].
    ///
    /// The optional per-item reads (presets, Core 1 mode, subharm solo, DAC
    /// mute, control interfaces, input source names) leave their defaults
    /// when the firmware does not answer them.
    ///
    /// Right after a connect that already decoded this connect's image
    /// (`Connection::fresh_image`, set by `attach_probed`), the platform and
    /// the image are not read again: the mirror's image fields and `compat`
    /// are kept and only the per-item values are read. The flag is consumed,
    /// so a later call reads everything.
    ///
    /// Ends by capturing the preset baseline when there is none for this
    /// device and its active preset (the first read after opening it, another
    /// device, another preset active). A re-read of the same device on the
    /// same preset (a refresh, a reconnect) keeps it, so unsaved changes stay
    /// unsaved until they are saved or reverted, and a change someone else
    /// made meanwhile is compared with the same baseline (owner, 2026-11-13).
    /// [`DspiCore::capture_baseline`] takes the live state as the baseline.
    ///
    /// Holds the core's lock for the whole read: a few hundred milliseconds
    /// over USB, seconds over MIDI or a slow Link. Every other call that
    /// takes the lock (`get_state`, setters, the RTA worker's polls) waits
    /// meanwhile; that also keeps a chunked bulk read from being torn down
    /// by another request.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open or when it
    /// went away; [`DspiError::Transfer`] (and the transport's other typed
    /// errors) when the device could not be talked to. An incompatible
    /// firmware is not an error: check `DspState.compat`.
    pub fn fetch_all(&self) -> Result<()> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let conn = inner.conn.as_mut().ok_or(DspiError::Disconnected)?;
        let (mut next, wire) = if conn.fresh_image {
            let mut next = inner.state.clone();
            // -- WP LK4 -- (review c82 F1) while a DSPi Link bind's background
            // reads fill them, the per-item values stay as they are: a reset
            // here would put defaults over what a user set meanwhile
            if conn.pending_probe.is_none() {
                reset_per_item(&mut next);
            }
            (next, conn.wire)
        } else {
            let mut next = state::default_state();
            let wire = read_platform_and_image(conn, &mut next)?;
            // -- WP LK4 -- (review c83 F1b) a later call in the window keeps
            // them too: the background reads still own them
            if conn.pending_probe.is_some() {
                carry_per_item(&inner.state, &mut next);
            }
            (next, wire)
        };
        // -- WP LK4 -- a DSPi Link bind still completing its probe reads
        // them in the background (and announces a refresh then)
        if wire.is_some() && conn.pending_probe.is_none() {
            read_per_item(conn, &mut next);
        }
        // -- WP SWAPFIX -- the journal's view of a re-read (journal.rs)
        let changed = inner.state.bulk_valid && next.bulk_raw != inner.state.bulk_raw;
        crate::journal::note_reread(conn, changed);
        conn.wire = wire;
        conn.fresh_image = false;
        inner.state = next;
        // -- WP SWAPFIX -- a re-read keeps the unsaved-changes baseline
        presets::keep_or_capture_baseline_locked(inner);
        Ok(())
    }

    /// Re-read the bulk image after the device changed state on its own (a
    /// preset load, a factory reset) and decode it into the mirror.
    ///
    /// Keeps the values the image does not carry (preset names, Core 1
    /// mode, ...) and the firmware identity. Re-judges the wire format, so
    /// the read-only mode follows the image. The mirror is left unchanged on
    /// failure.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open;
    /// [`DspiError::Transfer`] when the read fails or the image cannot be
    /// mapped onto this build's layout.
    pub fn refresh_params(&self) -> Result<()> {
        refresh_params_locked(&mut self.lock())
    }

    /// Write the whole cached state to the device in one bulk transfer:
    /// chunked `REQ_SET_ALL_PARAMS_CHUNK` (`0xA3`, at most 2048 bytes and
    /// the transport's `max_transfer` per chunk), or single-shot
    /// `REQ_SET_ALL_PARAMS` (`0xA1`) over a link without chunked bulk. The
    /// device applies it on its main loop once the last byte lands.
    ///
    /// The write is bracketed with the connection's echo filter, so the
    /// `BULK_INVALIDATED` it causes is recognised as our own. On success the
    /// mirror is re-decoded from the image that was sent (the codec's clamps
    /// applied).
    ///
    /// Runs under the transport's device lock (`Transport::lock`, a no-op
    /// except on DSPi Link, where another console's changes are held off
    /// for its duration; PLAN "Link device lock").
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open, or, like the
    /// old core, when no image has been read yet (nothing to apply: call
    /// [`DspiCore::fetch_all`] first); [`DspiError::ReadOnly`] when the
    /// firmware speaks another wire format; [`DspiError::Locked`] /
    /// [`DspiError::Denied`] when the Link hub refuses its lock (nothing is
    /// sent); the transport's typed error when a transfer fails (the device
    /// then applies nothing).
    pub fn apply_all_params(&self) -> Result<()> {
        check_apply_allowed(&mut self.lock())?;
        let lock = self.acquire_device_lock(LOCK_REASON)?;
        let result = {
            let mut guard = self.lock();
            if lock.is_current(&guard) {
                let r = apply_all_params_locked(&mut guard);
                // -- WP BULKFIX --
                r.map_err(|e| (e, bulk_left_muted(&guard)))
            } else {
                Err((DspiError::Disconnected, false))
            }
        };
        self.release_device_lock(lock);
        // -- WP BULKFIX -- (fixup bf F2) Logged with the lock released.
        result.map_err(|(e, left_muted)| {
            if left_muted {
                let e = with_why(e);
                self.log_left_muted("Applying the configuration", &e);
                return e;
            }
            e
        })
    }

    /// Read the meter block (`REQ_GET_STATUS` sub-query 9): the peak of each
    /// live wire channel (0..1), both cores' loads (%), the sticky clip
    /// flags (bit per wire channel) and the live input count.
    ///
    /// `peaks` always has `MAX_CHANNELS` (17) entries; those past
    /// `num_channels` are 0. `active_input_channels` is 0 from firmware that
    /// does not append it. Works on a read-only connection too.
    ///
    /// Over DSPi Link this is the latest frame of a `poll.subscribe` (no
    /// round trip, and it never delays a command): subscribed on the first
    /// call, unsubscribed about two seconds after the last, so stop calling
    /// when no meter is on screen. Until the first frame, or if the feed
    /// stalls, the block reads all zero with `stale` set. A hub that refuses the
    /// poll is read directly at a low rate; a repeat of the last such read is
    /// `stale` too.
    ///
    /// WP P0M: `peak_hold`, `peak_max` and `clip_hold` are the core's meter
    /// ballistics over these reads (`set_meter_ballistics`); a `stale` read
    /// leaves them as they were. When the clip hold ends the core clears the
    /// device's sticky flags itself (`REQ_CLEAR_CLIPS`, as `clear_clips`), so
    /// a client keeps no clip timer. A channel still clipping lights again on
    /// the next read; where the clear is refused (a viewer on a hub) the hold
    /// follows the device's sticky flags. New clips go to the event log.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open;
    /// [`DspiError::Transfer`] when the read fails or the reply is shorter
    /// than `num_channels * 2 + 6` bytes.
    pub fn fetch_status(&self) -> Result<SystemStatus> {
        // -- WP P0M --
        let mut status = self.fetch_status_raw()?;
        if self.events.meters(&mut status) {
            self.clear_clips_when_free();
        }
        Ok(status)
    }

    /// Copy of the cached state (the C ABI returns a borrowed pointer).
    /// Defaults while disconnected. Never fails. Under a named crossfeed
    /// preset, `crossfeed_freq` and `crossfeed_feed` are that preset's values
    /// (see `set_crossfeed_preset`).
    pub fn get_state(&self) -> DspState {
        // -- WP P1XF --
        crate::params::crossfeed_shown(self.lock().state.clone())
    }

    /// Longest delay the firmware's delay line holds, ms: 42 on RP2350
    /// (`platform_id` 1, 2048 samples at 48 kHz), 21 otherwise (RP2040,
    /// 1024 samples). Read from the mirror; never fails.
    pub fn max_delay_ms(&self) -> f32 {
        state::max_delay_ms_for(&self.lock().state)
    }

    /// Clear the device's sticky clip flags (`REQ_CLEAR_CLIPS`, which reads
    /// the flags back and clears them; the value is not used).
    ///
    /// Meter housekeeping, not a parameter: it touches no setting and
    /// nothing in the bulk image, so it is allowed on a read-only
    /// connection too. This is the one named exception (PLAN §1.2) to
    /// "state-changing opcodes only on a `Writable`": it goes out on a plain
    /// `Connection`.
    ///
    /// -- WP ROMODEF -- Not in the client read-only mode, which writes
    /// nothing, the device's counters included: [`DspiError::ReadOnly`],
    /// nothing sent, the clip hold left as it is.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device open; the
    /// transport's typed error when the transfer fails.
    pub fn clear_clips(&self) -> Result<()> {
        // -- WP ROMODEF -- (review romode F3)
        {
            let mut guard = self.lock();
            if guard.conn.is_some() {
                crate::read_only::check(&mut guard)?;
            }
        }
        // -- WP P0M -- the core's clip hold goes dark at once; a clear that
        // fails brings back what the device still reports
        self.events.clear_clip_hold();
        let sent = {
            let mut guard = self.lock();
            guard
                .conn()
                .and_then(|conn| conn.get_upto(op::REQ_CLEAR_CLIPS, 0, 4))
        };
        self.events.clear_result(sent.is_ok());
        sent.map(|_| ())
    }
}

// -- WP P0M --
impl DspiCore {
    /// [`DspiCore::fetch_status`]'s read, before the ballistics.
    fn fetch_status_raw(&self) -> Result<SystemStatus> {
        // WP LK2: over DSPi Link this never waits for the core lock, so the
        // meters keep going while `fetch_all`, `cs_fetch` or a re-read after a
        // resync runs on another thread (call it from a thread of its own).
        // When another call holds the lock, the meter path recorded with the
        // side channel serves. Elsewhere it takes the lock for its one
        // transfer. (A comment, not doc: the bindings' docs stay as they are.)
        let (n, side, old) = match self.try_lock() {
            Some(guard) => match self.status_locked(guard)? {
                Locked::Done(status) => return Ok(status),
                Locked::Side(n, side, old) => (n, side, old),
            },
            None => match self.meter_path() {
                Some(path) => (usize::from(path.num_channels), path.side, None),
                // USB, MIDI, the demo: their reads need the lock anyway
                None => match self.status_locked(self.lock())? {
                    Locked::Done(status) => return Ok(status),
                    Locked::Side(n, side, old) => (n, side, old),
                },
            },
        };
        drop(old);
        let n = n.min(MAX_CHANNELS);
        // n <= 17: at most 41 bytes.
        let want = n.saturating_mul(2).saturating_add(7);
        let len = u16::try_from(want).unwrap_or(u16::MAX);
        // DSPi Link: the hub's poll feed, with the core lock released. No
        // recent frame reads as zero, not as an error or a frozen meter, and is
        // flagged `stale`: it is not real silence.
        let read = side.meters(len)?;
        let data = read.data.unwrap_or_else(|| vec![0; want]);
        let mut status = parse_status(&data, n)
            .ok_or_else(|| short(op::REQ_GET_STATUS, want.saturating_sub(1), data.len()))?;
        status.stale = read.stale;
        status.age_ms = read.age_ms;
        Ok(status)
    }
}

// -- WP LK4 --
/// `wLength` of the meter block (`REQ_GET_STATUS` wValue 9) for
/// `num_channels`: what [`DspiCore::fetch_status`] asks for.
#[cfg(feature = "link")]
pub(crate) fn meter_len(num_channels: u8) -> u16 {
    let n = usize::from(num_channels).min(MAX_CHANNELS);
    // n <= 17: at most 41 bytes.
    u16::try_from(n.saturating_mul(2).saturating_add(7)).unwrap_or(u16::MAX)
}

// -- WP LK2 --
/// What [`DspiCore::fetch_status`] found with the lock held.
enum Locked {
    /// Read directly (USB, MIDI, the demo).
    Done(SystemStatus),
    /// Over the side channel, with this many channels, after the lock is
    /// released; and the meter path this replaced, to drop after it too.
    Side(usize, Arc<dyn crate::SideChannel>, Option<crate::MeterPath>),
}

impl DspiCore {
    /// [`DspiCore::fetch_status`] with the lock: a direct read, or the side
    /// channel to read with it released. Records the side channel's meter path.
    fn status_locked(&self, mut guard: std::sync::MutexGuard<'_, Inner>) -> Result<Locked> {
        let inner = &mut *guard;
        let n = usize::from(inner.state.num_channels).min(MAX_CHANNELS);
        let num_channels = inner.state.num_channels;
        let Some(conn) = inner.conn.as_mut() else {
            // -- WP LK4 -- a Link device being opened: its meter feed runs
            // from the image on, before the connection is installed
            drop(guard);
            let path = self.meter_path().ok_or(DspiError::Disconnected)?;
            return Ok(Locked::Side(
                usize::from(path.num_channels),
                path.side,
                None,
            ));
        };
        let Some(side) = conn.side.clone() else {
            // n <= 17: at most 41 bytes.
            let want = n.saturating_mul(2).saturating_add(7);
            let len = u16::try_from(want).unwrap_or(u16::MAX);
            let data = conn.get_upto(op::REQ_GET_STATUS, STATUS_METERS, len)?;
            return parse_status(&data, n)
                .map(Locked::Done)
                .ok_or_else(|| short(op::REQ_GET_STATUS, want.saturating_sub(1), data.len()));
        };
        let old = self.set_meter_path(Some(crate::MeterPath {
            num_channels,
            side: Arc::clone(&side),
        }));
        Ok(Locked::Side(n, side, old))
    }
}

// ── The connect-time read ─────────────────────────────────────────────

/// How a bulk read ended.
enum Image {
    /// The image was decoded into the state; how it was read.
    Read(WireRead),
    /// The device answered, but its image cannot be mapped onto V32 (older
    /// than V16, or a newer image whose prefix moved). `version` is the
    /// device's wire format (0 when the read was refused before it).
    Unmapped { version: u8 },
}

/// The platform and image part of `fetch_all`, on a fresh state. Returns
/// the connection's new `wire` (`None` when no image could be shown).
/// [`DspiCore::refresh_params`] for a flow that already holds the core's
/// lock, so nothing lands between the device's change and the re-read.
pub(crate) fn refresh_params_locked(inner: &mut Inner) -> Result<()> {
    let conn = inner.conn.as_mut().ok_or(DspiError::Disconnected)?;
    let mut next = inner.state.clone();
    match read_image(conn, &mut next)? {
        Image::Read(read) => {
            conn.wire = Some(read);
            inner.state = next;
            Ok(())
        }
        Image::Unmapped { version } => Err(DspiError::Transfer {
            msg: format!(
                "the device's settings (wire format {version}) cannot be read by this version"
            ),
        }),
    }
}

fn read_platform_and_image(conn: &mut Connection, s: &mut DspState) -> Result<Option<WireRead>> {
    if conn.vid == LEGACY_VID {
        // Pre-May-2026 firmware: far older than any wire format this build
        // maps, so its image is not even asked for (the old core's rule; it
        // also saves the busy retries a refused chunked read would cost).
        // It may answer differently; it is too old anyway. Only its being
        // gone is an error.
        if let Err(DspiError::Disconnected) = read_platform(conn, s) {
            return Err(DspiError::Disconnected);
        }
        s.compat = COMPAT_FIRMWARE_TOO_OLD;
        s.bulk_valid = false;
        return Ok(None);
    }
    read_platform(conn, s)?;
    match read_image(conn, s)? {
        Image::Read(read) => Ok(Some(read)),
        Image::Unmapped { version } => {
            if version != 0 {
                s.format_version = version;
            }
            s.compat = compat_of(version);
            s.bulk_valid = false;
            Ok(None)
        }
    }
}

/// Put the values the image does not carry back to their defaults, so a
/// failed per-item read cannot leave a previous device's value behind.
pub(crate) fn reset_per_item(s: &mut DspState) {
    let d = state::default_state();
    s.core1_mode = d.core1_mode;
    s.subharm_solo = d.subharm_solo;
    s.dac_mute_supported = d.dac_mute_supported;
    s.ctrl_iface_supported = d.ctrl_iface_supported;
    s.ctrl_status = d.ctrl_status;
    s.uart = d.uart;
    s.i2c = d.i2c;
    s.preset_occupied = d.preset_occupied;
    s.preset_startup_mode = d.preset_startup_mode;
    s.preset_default_slot = d.preset_default_slot;
    s.active_preset_slot = d.active_preset_slot;
    s.output_config_mode = d.output_config_mode;
    s.master_volume_mode = d.master_volume_mode;
    s.preset_names = d.preset_names;
}

/// The values the image does not carry. Each is optional: firmware that
/// lacks one stalls, and the default stays.
///
/// WP LK2: in two batches ([`dspi_transport::Transport::control_in_batch`]),
/// pipelined over DSPi Link (two windows of round trips instead of fifteen).
/// Elsewhere the transfers go one at a time in the order they always did: Core
/// 1 mode, subharm solo, DAC mute, the control interfaces' status, then their
/// UART and I2C configs only if the status answered, the preset directory and
/// the names. The one difference: a UART config that fails after the status
/// answered no longer skips the I2C read.
fn read_per_item(conn: &mut Connection, s: &mut DspState) {
    // -- WP LK4 -- the reads on any transport, the names applied here
    let names = crate::info::name_reads(conn);
    let answers = read_per_item_on(conn.transport(), &names, s);
    crate::info::apply_names(conn, &names, answers);
}

/// [`read_per_item`]'s reads on `t` into `s`, with the name reads `names`
/// in the second batch; their answers are returned, to apply to the
/// connection ([`crate::info::apply_names`]). WP LK4: a DSPi Link bind runs
/// it in the background, without the core lock.
pub(crate) fn read_per_item_on(
    t: &mut dyn dspi_transport::Transport,
    names: &[InRead],
    s: &mut DspState,
) -> Vec<Result<Vec<u8>>> {
    let first = t.control_in_batch(&[
        InRead::upto(op::REQ_GET_CORE1_MODE, 0, 1),
        InRead::upto(op::REQ_GET_SUBHARM_SOLO, 0, 1),
        InRead::upto(op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16),
        InRead::upto(op::REQ_GET_CTRL_IFACE_STATUS, 0, 8),
    ]);
    let mut first = first.into_iter().map(|r| r.map_err(DspiError::from));
    let mut next = || first.next().unwrap_or(Err(DspiError::Disconnected));
    if let Ok(d) = at_least(op::REQ_GET_CORE1_MODE, next(), 1) {
        apply_core1_mode(&d, s);
    }
    if let Ok(d) = at_least(op::REQ_GET_SUBHARM_SOLO, next(), 1) {
        apply_subharm_solo(&d, s);
    }
    if let Ok(d) = at_least(op::REQ_GET_DAC_HW_MUTE_CONFIG, next(), DAC_MUTE_REPLY) {
        apply_dac_mute(&d, s);
    }
    let status = at_least(op::REQ_GET_CTRL_IFACE_STATUS, next(), 5);
    if status.is_err() {
        s.ctrl_iface_supported = false;
    }

    let mut reads = Vec::new();
    if status.is_ok() {
        reads.push(InRead::upto(op::REQ_GET_UART_CONFIG, 0, 8));
        reads.push(InRead::upto(op::REQ_GET_I2C_CONFIG, 0, 8));
    }
    reads.push(InRead::upto(op::REQ_PRESET_GET_DIR, 0, 7));
    let name_len = u16::try_from(CHANNEL_NAME_LEN).unwrap_or(u16::MAX);
    let slots: Vec<u8> = (0..MAX_PRESETS)
        .filter_map(|slot| u8::try_from(slot).ok())
        .collect();
    reads.extend(
        slots
            .iter()
            .map(|slot| InRead::upto(op::REQ_PRESET_GET_NAME, u16::from(*slot), name_len)),
    );
    // -- WP LK4 -- the source and device names (WP CG13, QD4) in the same batch
    reads.extend(names.iter().copied());
    let second = t.control_in_batch(&reads);
    let mut second = second.into_iter().map(|r| r.map_err(DspiError::from));
    let mut next = || second.next().unwrap_or(Err(DspiError::Disconnected));
    if let Ok(st) = status {
        let u = at_least(op::REQ_GET_UART_CONFIG, next(), 8);
        let i = at_least(op::REQ_GET_I2C_CONFIG, next(), 4);
        if let (Ok(u), Ok(i)) = (u, i) {
            apply_ctrl_ifaces(&st, &u, &i, s);
        }
    }
    if let Ok(d) = at_least(op::REQ_PRESET_GET_DIR, next(), 5) {
        apply_preset_directory(&d, s);
    }
    for slot in slots {
        apply_preset_name(next().map(|d| name_from(&d)).as_ref(), s, slot);
    }
    // -- WP CG13 -- -- WP QD4 -- (batched since WP LK4)
    names.iter().map(|_| next()).collect()
}

// -- WP LK4 --
/// Carry the values the image does not carry ([`reset_per_item`]'s list) from
/// `from` into `to` as they are (review c83 F1b).
fn carry_per_item(from: &DspState, to: &mut DspState) {
    macro_rules! carry {
        ($($f:ident),*) => {$( to.$f = from.$f.clone(); )*};
    }
    carry!(
        core1_mode,
        subharm_solo,
        dac_mute_supported,
        ctrl_iface_supported,
        ctrl_status,
        uart,
        i2c,
        preset_occupied,
        preset_startup_mode,
        preset_default_slot,
        active_preset_slot,
        output_config_mode,
        master_volume_mode,
        preset_names
    );
}

// -- WP LK4 --
/// Fill the values the image does not carry ([`reset_per_item`]'s list) from
/// `read`, the background per-item reads of a DSPi Link bind, into `to`, the
/// mirror now; `before` is what the mirror held when those reads were issued.
/// Field by field (preset names name by name), only a value nobody changed
/// since takes the read one: a user's write or a notification that arrived
/// meanwhile is newer than the read and wins (review c82 F1). The rule
/// compares values, not writes (review c83 N2): a write of exactly the value
/// the mirror held (or A, then B, then A again) cannot be told from none, and
/// the read wins; a per-connection write count would be exact. Narrow: the
/// field then shows what the device answered moments before.
#[cfg(feature = "link")]
pub(crate) fn merge_per_item(before: &DspState, read: &DspState, to: &mut DspState) {
    macro_rules! fill {
        ($($f:ident),*) => {$(
            if to.$f == before.$f {
                to.$f = read.$f.clone();
            }
        )*};
    }
    fill!(
        core1_mode,
        subharm_solo,
        dac_mute_supported,
        ctrl_iface_supported,
        ctrl_status,
        uart,
        i2c,
        preset_occupied,
        preset_startup_mode,
        preset_default_slot,
        active_preset_slot,
        output_config_mode,
        master_volume_mode
    );
    for (i, name) in to.preset_names.iter_mut().enumerate() {
        if before.preset_names.get(i) == Some(name)
            && let Some(r) = read.preset_names.get(i)
        {
            name.clone_from(r);
        }
    }
}

/// `COMPAT_*` for a device wire format other than V32's (0: unknown, old).
fn compat_of(version: u8) -> u8 {
    if version > V32 {
        COMPAT_FIRMWARE_TOO_NEW
    } else {
        COMPAT_FIRMWARE_TOO_OLD
    }
}

/// Read the bulk image (any wire format) and decode it into `s`: the image
/// fields, `format_version` (the device's), `compat` and `bulk_valid`
/// (true only for an exact V32 image). `s` is untouched unless the result
/// is `Image::Read`.
///
/// Reads the raw bytes and maps them here rather than through
/// `compat::read_bulk_any`, which does both: its refusal of an unmappable
/// image does not say which wire format the device speaks, and `compat`
/// needs that (too old or too new).
fn read_image(conn: &mut Connection, s: &mut DspState) -> Result<Image> {
    let bytes = match dspi_session::compat::read_bulk_bytes(conn.transport()) {
        Ok(b) => b,
        // Refused on the first chunk: a header older than V16.
        Err(WriteError::Transport(TransportError::Incompatible(_))) => {
            return Ok(Image::Unmapped { version: 0 });
        }
        Err(e) => return Err(e.into()),
    };
    let version = bytes.first().copied().unwrap_or(0);
    let normalized = match wire_compat::normalize(&bytes) {
        Ok(n) => n,
        Err(WireError::UnsupportedVersion { got, .. }) => {
            return Ok(Image::Unmapped { version: got });
        }
        // Too short for its version, or a header that does not decode: not
        // something this build can show.
        Err(_) => return Ok(Image::Unmapped { version }),
    };
    let mut next = s.clone();
    if state::decode_bulk(normalized.packet.as_bytes(), &mut next).is_err() {
        // `normalize` always yields a V32-sized, V32-headed image.
        return Ok(Image::Unmapped { version });
    }
    next.format_version = normalized.device_version;
    next.compat = match normalized.read {
        WireRead::Exact => COMPAT_OK,
        WireRead::Older => COMPAT_FIRMWARE_TOO_OLD,
        WireRead::Newer => COMPAT_FIRMWARE_TOO_NEW,
    };
    // Display only unless it is this build's own format (PLAN §1.5).
    next.bulk_valid = normalized.is_writable();
    *s = next;
    Ok(Image::Read(normalized.read))
}

/// Send `image` as one bulk write: chunked `0xA3` where the link allows it,
/// else single-shot `0xA1`. Not retried: a failed chunk closes the device's
/// chunk session, and a retried `0xA1` could apply twice.
fn write_image(w: &mut Writable<'_>, image: &[u8]) -> Result<()> {
    let (chunked, max) = {
        let t = w.conn().transport();
        (t.caps().chunked_bulk, t.max_transfer())
    };
    if !chunked {
        return w.set(op::REQ_SET_ALL_PARAMS, 0, image);
    }
    let size = BULK_WRITE_CHUNK.min(max).max(1);
    let mut offset = 0usize;
    for chunk in image.chunks(size) {
        let value = u16::try_from(offset).map_err(|_| DspiError::InvalidArgument {
            msg: format!("a bulk image of {} bytes does not fit wValue", image.len()),
        })?;
        w.set(op::REQ_SET_ALL_PARAMS_CHUNK, value, chunk)?;
        offset = offset.saturating_add(chunk.len());
    }
    Ok(())
}

// ── Per-item reads (values the image does not carry) ──────────────────

/// A short-reply error in the transport's words.
fn short(opcode: u8, wanted: usize, got: usize) -> DspiError {
    TransportError::ShortRead {
        opcode,
        wanted,
        got,
    }
    .into()
}

/// An IN transfer of up to `len` bytes that must bring at least `min`.
fn read_min(
    conn: &mut Connection,
    opcode: u8,
    value: u16,
    len: u16,
    min: usize,
) -> Result<Vec<u8>> {
    at_least(opcode, conn.get_upto(opcode, value, len), min)
}

/// A reply that must bring at least `min` bytes.
fn at_least(opcode: u8, reply: Result<Vec<u8>>, min: usize) -> Result<Vec<u8>> {
    let d = reply?;
    if d.len() < min {
        return Err(short(opcode, min, d.len()));
    }
    Ok(d)
}

/// The byte at `at`, 0 past the end (callers check the length first).
fn byte(d: &[u8], at: usize) -> u8 {
    d.get(at).copied().unwrap_or(0)
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([byte(d, at), byte(d, at.saturating_add(1))])
}

fn u32_at(d: &[u8], at: usize) -> u32 {
    let b = |k: usize| byte(d, at.saturating_add(k));
    u32::from_le_bytes([b(0), b(1), b(2), b(3)])
}

/// `REQ_GET_PLATFORM` into the platform, output count and firmware version.
pub(crate) fn read_platform(conn: &mut Connection, s: &mut DspState) -> Result<()> {
    let d = read_min(conn, op::REQ_GET_PLATFORM, 0, PLATFORM_LEN, 4)?;
    state::apply_platform(&d, s).map_err(|_| short(op::REQ_GET_PLATFORM, 4, d.len()))
}

/// `REQ_GET_CORE1_MODE` into `core1_mode`.
pub(crate) fn read_core1_mode(conn: &mut Connection, s: &mut DspState) -> Result<u8> {
    let d = read_min(conn, op::REQ_GET_CORE1_MODE, 0, 1, 1)?;
    Ok(apply_core1_mode(&d, s))
}

fn apply_core1_mode(d: &[u8], s: &mut DspState) -> u8 {
    s.core1_mode = byte(d, 0);
    s.core1_mode
}

/// A `REQ_GET_SUBHARM_SOLO` reply into `subharm_solo`.
fn apply_subharm_solo(d: &[u8], s: &mut DspState) -> bool {
    s.subharm_solo = byte(d, 0) != 0;
    s.subharm_solo
}

/// `REQ_GET_DAC_HW_MUTE_CONFIG` (8 bytes): marks the DAC mute supported and
/// takes the device's live config. When the image is writable its 8 bytes
/// are patched into `bulk_raw` at the DAC mute section (a later bulk write
/// would re-apply them); nothing else in `bulk_raw` changes.
pub(crate) fn read_dac_mute(conn: &mut Connection, s: &mut DspState) -> Result<()> {
    let d = read_min(conn, op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16, DAC_MUTE_REPLY)?;
    apply_dac_mute(&d, s);
    Ok(())
}

/// [`read_dac_mute`]'s reply (at least [`DAC_MUTE_REPLY`] bytes) into `s`.
fn apply_dac_mute(d: &[u8], s: &mut DspState) {
    s.dac_mute_supported = true;
    s.dac_mute_enabled = byte(d, 0) != 0;
    s.dac_mute_active_low = byte(d, 1) != 0;
    s.dac_mute_pin = byte(d, 2);
    s.dac_mute_hold_ms = u16_at(d, 4);
    s.dac_mute_release_ms = u16_at(d, 6);
    if s.bulk_valid {
        let at = dspi_proto::generated::OFF_DAC_HW_MUTE;
        let n = DAC_MUTE_REPLY.min(dspi_proto::generated::LEN_DAC_HW_MUTE);
        if let (Some(dst), Some(src)) = (
            at.checked_add(n)
                .and_then(|end| s.bulk_raw.get_mut(at..end)),
            d.get(..n),
        ) {
            dst.copy_from_slice(src);
        }
    }
}

/// The control interfaces: status (5 bytes), UART config (8) and I2C
/// config (4). `ctrl_iface_supported` is false when the status read fails,
/// true once all three answered; nothing else changes unless they do.
pub(crate) fn read_ctrl_ifaces(conn: &mut Connection, s: &mut DspState) -> Result<()> {
    let st = match read_min(conn, op::REQ_GET_CTRL_IFACE_STATUS, 0, 8, 5) {
        Ok(d) => d,
        Err(e) => {
            s.ctrl_iface_supported = false;
            return Err(e);
        }
    };
    let u = read_min(conn, op::REQ_GET_UART_CONFIG, 0, 8, 8)?;
    let i = read_min(conn, op::REQ_GET_I2C_CONFIG, 0, 8, 4)?;
    apply_ctrl_ifaces(&st, &u, &i, s);
    Ok(())
}

/// The three replies of [`read_ctrl_ifaces`] (status, UART, I2C) into `s`.
fn apply_ctrl_ifaces(st: &[u8], u: &[u8], i: &[u8], s: &mut DspState) {
    s.ctrl_iface_supported = true;
    s.ctrl_status = CtrlIfaceStatus {
        uart_last_status: byte(st, 0),
        uart_live: byte(st, 1) != 0,
        i2c_last_status: byte(st, 2),
        i2c_live: byte(st, 3) != 0,
        protocol_version: byte(st, 4),
    };
    s.uart = UartConfig {
        enabled: byte(u, 0) != 0,
        tx_pin: byte(u, 1),
        rx_pin: byte(u, 2),
        notify: byte(u, 3) != 0,
        baud: u32_at(u, 4),
    };
    s.i2c = I2cConfig {
        enabled: byte(i, 0) != 0,
        sda_pin: byte(i, 1),
        scl_pin: byte(i, 2),
        address: byte(i, 3),
    };
}

/// `REQ_PRESET_GET_DIR` (5 to 7 bytes) into the preset fields, the active
/// slot and the two persistence modes (defaults 1 and 0 from firmware that
/// answers 5 bytes).
pub(crate) fn read_preset_directory(
    conn: &mut Connection,
    s: &mut DspState,
) -> Result<PresetDirectory> {
    let d = read_min(conn, op::REQ_PRESET_GET_DIR, 0, 7, 5)?;
    Ok(apply_preset_directory(&d, s))
}

/// A `REQ_PRESET_GET_DIR` reply (at least 5 bytes) into `s`.
fn apply_preset_directory(d: &[u8], s: &mut DspState) -> PresetDirectory {
    let dir = PresetDirectory {
        occupied_mask: u16_at(d, 0),
        startup_mode: byte(d, 2),
        default_slot: byte(d, 3),
        last_active: byte(d, 4),
        output_config_mode: d.get(5).copied().unwrap_or(1),
        master_volume_mode: d.get(6).copied().unwrap_or(0),
    };
    s.preset_occupied = dir.occupied_mask;
    s.preset_startup_mode = dir.startup_mode;
    s.preset_default_slot = dir.default_slot;
    s.active_preset_slot = dir.last_active;
    s.output_config_mode = dir.output_config_mode;
    s.master_volume_mode = dir.master_volume_mode;
    dir
}

/// `REQ_PRESET_GET_NAME` of `slot` into `preset_names[slot]`. An empty slot
/// stalls: its name becomes empty and the error is returned.
/// [`DspiError::InvalidArgument`] for a slot past `MAX_PRESETS`.
pub(crate) fn read_preset_name(
    conn: &mut Connection,
    s: &mut DspState,
    slot: u8,
) -> Result<String> {
    let index = usize::from(slot);
    if index >= MAX_PRESETS {
        return Err(DspiError::InvalidArgument {
            msg: format!("preset slot {slot} is out of range (0..{MAX_PRESETS})"),
        });
    }
    let len = u16::try_from(CHANNEL_NAME_LEN).unwrap_or(u16::MAX);
    let name = conn
        .get_upto(op::REQ_PRESET_GET_NAME, u16::from(slot), len)
        .map(|d| name_from(&d));
    apply_preset_name(name.as_ref(), s, slot);
    name
}

/// `slot`'s name read, or empty when the read failed (an empty slot stalls).
/// A slot past `MAX_PRESETS` is ignored.
fn apply_preset_name(name: std::result::Result<&String, &DspiError>, s: &mut DspState, slot: u8) {
    if s.preset_names.len() < MAX_PRESETS {
        s.preset_names.resize(MAX_PRESETS, String::new());
    }
    if let Some(n) = s.preset_names.get_mut(usize::from(slot)) {
        match name {
            Ok(v) => n.clone_from(v),
            Err(_) => n.clear(),
        }
    }
}

/// A name buffer: UTF-8 (lossy) up to the first NUL, at most 31 bytes.
fn name_from(d: &[u8]) -> String {
    let max = CHANNEL_NAME_LEN.saturating_sub(1);
    let d = d.get(..d.len().min(max)).unwrap_or(&[]);
    let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
    String::from_utf8_lossy(d.get(..end).unwrap_or(&[])).into_owned()
}

/// Parse the meter block for `n` live channels: `peaks[n]` (u16, Q15),
/// `cpu0`, `cpu1`, `clip_flags` (u32), then `active_inputs` if present.
/// `None` if shorter than `n * 2 + 6` bytes.
fn parse_status(data: &[u8], n: usize) -> Option<SystemStatus> {
    let n = n.min(MAX_CHANNELS);
    let base = n.checked_mul(2)?;
    if data.len() < base.checked_add(6)? {
        return None;
    }
    let mut peaks = vec![0.0f32; MAX_CHANNELS];
    for (i, p) in peaks.iter_mut().enumerate().take(n) {
        *p = f32::from(u16_at(data, i.saturating_mul(2))) / 32767.0;
    }
    Some(SystemStatus {
        peaks,
        cpu0: byte(data, base),
        cpu1: byte(data, base.saturating_add(1)),
        clip_flags: u32_at(data, base.saturating_add(2)),
        active_input_channels: byte(data, base.saturating_add(6)),
        // n <= MAX_CHANNELS (17).
        num_channels: u8::try_from(n).unwrap_or(u8::MAX),
        ..SystemStatus::default()
    })
}

/// [`DspiCore::apply_all_params`]'s refusals, before any lock or transfer.
fn check_apply_allowed(inner: &mut Inner) -> Result<()> {
    let wire = inner.conn()?.wire;
    crate::read_only::check(inner)?; // -- WP ROMODE --
    if matches!(
        inner.state.compat,
        COMPAT_FIRMWARE_TOO_OLD | COMPAT_FIRMWARE_TOO_NEW
    ) || matches!(wire, Some(WireRead::Older | WireRead::Newer))
    {
        // -- WP QD6 --
        inner.read_only_refusals = inner.read_only_refusals.saturating_add(1);
        return Err(DspiError::ReadOnly);
    }
    if !inner.state.bulk_valid || wire.is_none() {
        return Err(DspiError::Disconnected);
    }
    Ok(())
}

/// [`DspiCore::apply_all_params`] with the core lock already held, so a
/// caller can change the mirror and write it in one critical section (WP
/// C13's `config_import`). The caller holds the device lock
/// ([`LOCK_REASON`], [`DspiCore::acquire_device_lock`]): it cannot be waited
/// for here, under the core lock. Same contract and errors.
pub(crate) fn apply_all_params_locked(inner: &mut Inner) -> Result<()> {
    check_apply_allowed(inner)?;
    let image = state::encode_bulk(&inner.state);
    let mut w = inner.writable_conn()?;
    // -- WP BULKFIX --
    w.conn().bulk_left_muted = false;
    let repair = bulk_gain_repair(w.conn(), &image);

    w.conn().echo.bulk_write_started(Instant::now());
    // -- WP BULKFIX --
    let mut unmuted = 0usize;
    let (sent, left_muted) = match &repair {
        None => (write_image(&mut w, &image), false),
        Some(r) => match write_image(&mut w, &r.muted) {
            Ok(()) => {
                let repaired = repair_after_write(&mut w, r, &mut unmuted);
                let failed = repaired.is_err();
                (repaired, failed)
            }
            Err(e) => {
                // A lost reply may hide an applied image: its gains are unknown.
                let all = dspi_session::bulk_gain::all_outputs(r.outputs);
                w.conn().gain_guard.suspect(all);
                (Err(e), false)
            }
        },
    };
    w.conn()
        .echo
        .bulk_write_finished(Instant::now(), sent.is_ok());
    if let (true, Some(r)) = (left_muted, &repair) {
        // -- fixup bf (F2) -- The device holds the muted image: so does the
        // mirror, and `bulk_left_muted` says why to the caller.
        w.conn().bulk_left_muted = true;
        let _ = state::decode_bulk(&r.muted, &mut inner.state);
        for u in r.unmutes.iter().take(unmuted) {
            if let Some(m) = inner.state.output_muted.get_mut(usize::from(u.value)) {
                *m = false;
            }
        }
    }
    sent?;

    // Cannot fail: `encode_bulk` always yields a BULK_SIZE V32 image.
    // Were it to, the mirror simply keeps the state that was sent.
    let _ = state::decode_bulk(&image, &mut inner.state);
    // -- WP P0W -- the whole state replaced: the history before it is gone
    crate::journal::journal(inner, "bulk", &[], dspi_proto::value::Value::Trigger);
    Ok(())
}

// -- WP BULKFIX -- (fixup bf F2)
/// Whether the last bulk write on the open connection left the outputs muted.
pub(crate) fn bulk_left_muted(inner: &Inner) -> bool {
    inner.conn.as_ref().is_some_and(|c| c.bulk_left_muted)
}

/// Why the outputs are muted after a failed bulk write, for an error line.
pub(crate) const LEFT_MUTED: &str = "the device's outputs were left muted, because the gains it \
     was sent could not be checked or corrected (its firmware plays a bulk write's gains wrong \
     until they are re-sent); apply the configuration again or load a preset";

/// `e` with [`LEFT_MUTED`] in its message. Other kinds (a lost device, a
/// Link refusal) keep theirs, so a client still tells them apart.
pub(crate) fn with_why(e: DspiError) -> DspiError {
    match e {
        DspiError::Transfer { msg } => DspiError::Transfer {
            msg: format!("{msg}; {LEFT_MUTED}"),
        },
        other => other,
    }
}

impl DspiCore {
    /// Record a bulk write whose repair failed in the event history. Call
    /// with the lock released.
    pub(crate) fn log_left_muted(&self, what: &str, e: &DspiError) {
        let text = match e {
            DspiError::Transfer { .. } => format!("{what} failed: {e}"),
            _ => format!("{what} failed ({e}): {LEFT_MUTED}"),
        };
        let r = dspi_session::events::EventRecord::new(
            dspi_session::events::EventKind::Import,
            crate::events::now_ms(),
            text,
        );
        self.events.record_all(vec![r]);
    }
}

// -- WP BULKFIX --
/// The repair a bulk write of `image` needs on this connection: `None` when
/// the firmware reports the bulk dB fix (`EXT_FIRMWARE_FIXES`, asked once per
/// connection, and only when `EXT_LIST` named it) or the image is not one the
/// firmware applies. Firmware v1.1.6 and every fork build without FWX's FW2
/// convert a bulk SET's gains with a Taylor series that plays deep cuts loud
/// ([`dspi_proto::bulk_gain`]).
fn bulk_gain_repair(
    conn: &mut Connection,
    image: &[u8],
) -> Option<dspi_proto::bulk_gain::GainRepair> {
    let guard = conn.gain_guard.clone();
    // A Link bind's probe may not have read `EXT_LIST` yet: then ask.
    let supported = conn
        .pending_probe
        .is_none()
        .then(|| conn.ext.supported.clone());
    if guard.fixed(conn.transport(), supported.as_deref()) {
        return None;
    }
    dspi_proto::bulk_gain::plan(image)
}

// -- WP BULKFIX -- (fixup bf F5: the writes go through `Writable`)
/// After the muted image was sent: wait for its apply and check the read-back,
/// re-send the gains, then unmute. Every output is suspect if this fails
/// before the gains are all in ([`dspi_session::bulk_gain::GainGuard`]).
/// `unmuted` counts the unmutes sent.
fn repair_after_write(
    w: &mut Writable<'_>,
    r: &dspi_proto::bulk_gain::GainRepair,
    unmuted: &mut usize,
) -> Result<()> {
    use dspi_session::bulk_gain::{all_outputs, check_readback, wait_applied};
    let guard = w.conn().gain_guard.clone();
    let gains = (|| -> Result<()> {
        let back = wait_applied(w.conn().transport())?;
        check_readback(&back, r)?;
        for g in &r.gains {
            w.set(g.opcode, g.value, &g.payload)?;
        }
        Ok(())
    })();
    if gains.is_err() {
        guard.suspect(all_outputs(r.outputs));
    } else {
        guard.clear_all();
    }
    gains?;
    for u in &r.unmutes {
        w.set(u.opcode, u.value, &u.payload)?;
        *unmuted = unmuted.saturating_add(1);
    }
    Ok(())
}

// -- WP BULKFIX --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_bulkfix;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c2 {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use dspi_proto::generated::notify as nfy;
    use dspi_transport::mock::Reply;
    use dspi_transport::{
        Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, MockTransport,
        NotificationSource, Transport, TransportCaps, TransportKind,
    };

    use super::*;

    fn demo_bulk() -> Vec<u8> {
        Capture::demo().bulk
    }

    fn decoded(image: &[u8]) -> DspState {
        let mut s = state::default_state();
        state::decode_bulk(image, &mut s).unwrap();
        s
    }

    /// A probed demo device, read with `fetch_all`.
    fn demo_core() -> (Arc<DspiCore>, FakeRemote) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        core.fetch_all().unwrap();
        (core, remote)
    }

    /// A device answering `bulk` (any wire format), attached bare: the
    /// strict base probe refuses anything but V32.
    fn fake_with(bulk: Vec<u8>, vid: u16) -> (Arc<DspiCore>, FakeRemote) {
        let cap = Capture {
            platform: Capture::demo().platform,
            bulk,
        };
        let dev = FakeDevice::new(cap, FakeOptions::default());
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach(Box::new(dev), "OLD".into(), vid);
        (core, remote)
    }

    /// The demo image relabelled as wire format `v`, cut or extended to
    /// `len` bytes.
    fn image_as(v: u8, len: usize) -> Vec<u8> {
        let mut b = demo_bulk();
        b.resize(len, 0);
        b[0] = v;
        b[6..8].copy_from_slice(&(len as u16).to_le_bytes());
        b
    }

    fn wire(core: &DspiCore) -> Option<WireRead> {
        core.lock().conn().unwrap().wire
    }

    #[test]
    fn wp_c2_fetch_all_decodes_the_demo_device() {
        let (core, _r) = demo_core();
        let s = core.get_state();
        let want = decoded(&demo_bulk());
        assert_eq!(s.compat, COMPAT_OK);
        assert_eq!(s.format_version, 32);
        assert!(s.bulk_valid);
        assert_eq!(s.bulk_raw.len(), dspi_proto::generated::BULK_SIZE);
        // The DAC mute read patches its section only: still the device's image.
        assert_eq!(s.bulk_raw, _r.image());
        assert_eq!(s.master_volume_db, want.master_volume_db);
        assert_eq!(s.filters, want.filters);
        assert_eq!(s.channel_names, want.channel_names);
        assert_eq!(s.platform_id, 1);
        assert_eq!(wire(&core), Some(WireRead::Exact));
        // Per-item values the fake answers.
        assert!(s.dac_mute_supported);
        assert_eq!(s.preset_names.len(), MAX_PRESETS);
        // The fake stalls the control-interface status.
        assert!(!s.ctrl_iface_supported);
        assert!(core.lock().writable_conn().is_ok());
    }

    #[test]
    fn wp_c2_fetch_all_rebuilds_the_mirror_from_the_device() {
        let dev = FakeDevice::demo();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        // Left from "a previous device": must not survive.
        core.lock().state.preset_names[3] = "stale".into();
        core.lock().state.core1_mode = 9;
        core.fetch_all().unwrap();
        let s = core.get_state();
        let mut want = state::default_state();
        state::apply_platform(&Capture::demo().platform, &mut want).unwrap();
        assert_eq!(
            (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
            (want.fw_major, want.fw_minor, want.fw_patch, want.fw_beta)
        );
        assert_ne!(s.preset_names[3], "stale");
        // The fake stalls the Core 1 mode read: the default stays.
        assert_eq!(s.core1_mode, 0);
        assert_eq!(s.output_config_mode, 1);
    }

    #[test]
    fn wp_c2_without_a_device_everything_is_disconnected() {
        let core = DspiCore::new();
        assert_eq!(core.fetch_all(), Err(DspiError::Disconnected));
        assert_eq!(core.refresh_params(), Err(DspiError::Disconnected));
        assert_eq!(core.apply_all_params(), Err(DspiError::Disconnected));
        assert_eq!(core.fetch_status().unwrap_err(), DspiError::Disconnected);
        assert_eq!(core.clear_clips(), Err(DspiError::Disconnected));
        assert_eq!(core.get_state(), state::default_state());
    }

    #[test]
    fn wp_c2_an_older_image_is_shown_read_only() {
        let size = wire_compat::bulk_size(31).unwrap();
        let (core, remote) = fake_with(image_as(31, size), 0);
        let before = remote.image();
        core.fetch_all().unwrap();
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD);
        assert_eq!(s.format_version, 31);
        assert!(!s.bulk_valid);
        assert_eq!(wire(&core), Some(WireRead::Older));
        // Shown: the fields both layouts share come through the mapping.
        let v32 = decoded(&demo_bulk());
        assert_eq!(s.master_volume_db, v32.master_volume_db);
        assert_eq!(s.channel_names, v32.channel_names);
        // Never written back.
        assert_eq!(core.apply_all_params(), Err(DspiError::ReadOnly));
        assert!(matches!(
            core.lock().writable_conn(),
            Err(DspiError::ReadOnly)
        ));
        assert_eq!(remote.image(), before);
        // Reads still work.
        core.refresh_params().unwrap();
        assert_eq!(core.get_state().compat, COMPAT_FIRMWARE_TOO_OLD);
        core.fetch_status().unwrap();
    }

    #[test]
    fn wp_c2_a_newer_image_shows_its_v32_prefix_read_only() {
        let (core, remote) = fake_with(image_as(33, 6200), 0);
        core.fetch_all().unwrap();
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_NEW);
        assert_eq!(s.format_version, 33);
        assert!(!s.bulk_valid);
        assert_eq!(wire(&core), Some(WireRead::Newer));
        assert_eq!(s.master_volume_db, decoded(&demo_bulk()).master_volume_db);
        let before = remote.image();
        assert_eq!(core.apply_all_params(), Err(DspiError::ReadOnly));
        assert_eq!(remote.image(), before);
    }

    #[test]
    fn wp_c2_an_unmappable_image_is_judged_not_an_error() {
        // Older than V16: refused on the first chunk.
        let (core, _r) = fake_with(image_as(15, 5000), 0);
        core.fetch_all().unwrap();
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD);
        assert!(!s.bulk_valid);
        assert_eq!(wire(&core), None);
        assert_eq!(core.apply_all_params(), Err(DspiError::ReadOnly));
        assert!(matches!(
            core.refresh_params(),
            Err(DspiError::Transfer { .. })
        ));

        // Newer, but its V32 prefix moved (header byte 5 is not 12).
        let mut b = image_as(34, 6200);
        b[5] = 0;
        let (core, _r) = fake_with(b, 0);
        core.fetch_all().unwrap();
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_NEW);
        assert_eq!(s.format_version, 34);
        assert_eq!(wire(&core), None);
        assert_eq!(core.apply_all_params(), Err(DspiError::ReadOnly));
    }

    #[test]
    fn wp_c2_legacy_firmware_that_cannot_be_read_is_too_old() {
        let device = || {
            MockTransport::new()
                .data(op::REQ_GET_PLATFORM, vec![0u8, 1, 0x05, 2])
                .reply(
                    op::REQ_GET_ALL_PARAMS_CHUNK,
                    Reply::Error(TransportError::Usb("pipe error".into())),
                )
        };
        let core = DspiCore::new();
        let legacy = device();
        let log = legacy.log_handle();
        core.attach(Box::new(legacy), "LEGACY".into(), LEGACY_VID);
        core.fetch_all().unwrap();
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD);
        assert!(!s.bulk_valid);
        assert_eq!((s.fw_major, s.fw_minor, s.fw_patch), (1, 0, 5));
        // The image is not even asked for, and nothing else is read.
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .map(|e| e.opcode)
                .collect::<Vec<_>>(),
            vec![op::REQ_GET_PLATFORM]
        );
        assert_eq!(core.apply_all_params(), Err(DspiError::ReadOnly));

        // The same device under the current VID could not be talked to.
        let core = DspiCore::new();
        core.attach(Box::new(device()), "X".into(), 0x2E8B);
        assert!(matches!(core.fetch_all(), Err(DspiError::Transfer { .. })));
        assert_eq!(
            core.get_state(),
            state::default_state(),
            "unchanged on failure"
        );
    }

    #[test]
    fn wp_c2_a_short_platform_reply_is_an_error_not_a_panic() {
        let t = MockTransport::new().data(op::REQ_GET_PLATFORM, vec![1u8, 1]);
        let core = DspiCore::new();
        core.attach(Box::new(t), "X".into(), 0);
        assert!(matches!(core.fetch_all(), Err(DspiError::Transfer { .. })));
    }

    #[test]
    fn wp_c2_apply_all_params_writes_the_mirror_back() {
        let (core, remote) = demo_core();
        core.lock().state.master_volume_db = -33.5;
        core.apply_all_params().unwrap();
        assert_eq!(decoded(&remote.image()).master_volume_db, -33.5);
        assert_eq!(core.get_state().master_volume_db, -33.5);
        assert_eq!(remote.image(), core.get_state().bulk_raw);
    }

    /// A baseline exists once the app has connected, so `preset_dirty` means
    /// something from the first edit on.
    #[test]
    fn wp_b2_fetch_all_leaves_a_baseline() {
        let (core, _remote) = demo_core();
        core.fetch_all().unwrap();
        assert!(!core.preset_dirty());
        core.set_bypass(!core.get_state().bypass).unwrap();
        assert!(core.preset_dirty());
    }

    #[test]
    fn wp_c2_apply_all_params_brackets_the_write_for_the_echo_filter() {
        let (core, _remote) = demo_core();
        core.apply_all_params().unwrap();
        let mut inner = core.lock();
        let conn = inner.conn().unwrap();
        let now = Instant::now();
        assert!(conn.echo.own_bulk_pending(now));
        let bulk = nfy::PARAM_SRC_BULK_SET as u8;
        assert!(
            conn.echo
                .classify_invalidation(now, TransportKind::Fake, bulk, None, None)
        );
        // Claimed once: a second invalidation is another host's.
        assert!(
            !conn
                .echo
                .classify_invalidation(now, TransportKind::Fake, bulk, None, None)
        );
        assert!(!conn.echo.own_bulk_pending(now + Duration::from_secs(2)));
    }

    #[test]
    fn wp_c2_a_failed_apply_leaves_no_pending_echo() {
        let (core, _r) = demo_core();
        // Swap in a transport that stalls the bulk write, keeping the
        // connection's read state.
        {
            let mut inner = core.lock();
            inner.conn().unwrap().link = crate::Wire::Raw(Box::new(
                MockTransport::new().reply(op::REQ_SET_ALL_PARAMS_CHUNK, Reply::Stall),
            ));
        }
        assert!(matches!(
            core.apply_all_params(),
            Err(DspiError::Transfer { .. })
        ));
        let mut inner = core.lock();
        assert!(!inner.conn().unwrap().echo.own_bulk_pending(Instant::now()));
    }

    #[test]
    fn wp_c2_apply_before_any_read_is_disconnected() {
        let core = DspiCore::new();
        core.attach(Box::new(FakeDevice::demo()), "DEMO".into(), 0);
        assert_eq!(core.apply_all_params(), Err(DspiError::Disconnected));
    }

    #[test]
    fn wp_c2_bulk_writes_respect_max_transfer() {
        let opts = FakeOptions {
            max_transfer: Some(1024),
            ..FakeOptions::default()
        };
        let dev = FakeDevice::new(Capture::demo(), opts);
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach(Box::new(dev), "DEMO".into(), 0);
        core.fetch_all().unwrap();
        core.lock().state.master_volume_db = -40.0;
        core.apply_all_params().unwrap();
        assert_eq!(decoded(&remote.image()).master_volume_db, -40.0);
    }

    /// A FakeDevice behind a recording transport: optionally Link-like (no
    /// chunked bulk), optionally refusing the device lock. Logs every
    /// transfer as `"in|out <opcode> <wValue>"` and the lock calls.
    struct Spy {
        inner: FakeDevice,
        log: Arc<Mutex<Vec<String>>>,
        link: bool,
        refuse_lock: Option<TransportError>,
    }

    impl Spy {
        fn new(inner: FakeDevice, link: bool) -> (Self, Arc<Mutex<Vec<String>>>) {
            let log = Arc::new(Mutex::new(Vec::new()));
            let spy = Self {
                inner,
                log: Arc::clone(&log),
                link,
                refuse_lock: None,
            };
            (spy, log)
        }
        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }
    }

    fn count(log: &Arc<Mutex<Vec<String>>>, entry: &str) -> usize {
        log.lock().unwrap().iter().filter(|e| *e == entry).count()
    }

    fn has_op(log: &Arc<Mutex<Vec<String>>>, dir: &str, opcode: u8) -> bool {
        let prefix = format!("{dir} {opcode:02X} ");
        log.lock().unwrap().iter().any(|e| e.starts_with(&prefix))
    }

    impl Transport for Spy {
        fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
            self.note(format!("in {op:02X} {value}"));
            self.inner.control_in(op, value, len)
        }
        fn control_in_upto(
            &mut self,
            op: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.note(format!("in {op:02X} {value}"));
            self.inner.control_in_upto(op, value, len)
        }
        fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
            self.note(format!("out {op:02X} {value}"));
            self.inner.control_out(op, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.inner.descriptor()
        }
        fn max_transfer(&self) -> usize {
            if self.link {
                8192
            } else {
                self.inner.max_transfer()
            }
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            if self.link {
                TransportCaps {
                    kind: TransportKind::Link,
                    chunked_bulk: false,
                    remote: true,
                }
            } else {
                self.inner.caps()
            }
        }
        fn lock(&mut self, reason: &str, timeout: Duration) -> dspi_transport::Result<()> {
            self.note(format!("lock {reason} {}", timeout.as_secs()));
            match &self.refuse_lock {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
        fn unlock(&mut self) {
            self.note("unlock".into());
        }
    }

    #[test]
    fn wp_c2_over_link_bulk_goes_single_shot() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let (t, log) = Spy::new(dev, true);
        let core = DspiCore::new();
        core.attach(Box::new(t), "LINK".into(), 0);
        core.fetch_all().unwrap();
        assert_eq!(core.get_state().compat, COMPAT_OK);
        core.lock().state.master_volume_db = -12.0;
        core.apply_all_params().unwrap();
        assert!(has_op(&log, "in", op::REQ_GET_ALL_PARAMS));
        assert!(has_op(&log, "out", op::REQ_SET_ALL_PARAMS));
        assert!(!has_op(&log, "in", op::REQ_GET_ALL_PARAMS_CHUNK));
        assert!(!has_op(&log, "out", op::REQ_SET_ALL_PARAMS_CHUNK));
        assert_eq!(decoded(&remote.image()).master_volume_db, -12.0);
    }

    #[test]
    fn wp_c2_apply_all_params_runs_under_the_link_lock() {
        let (t, log) = Spy::new(FakeDevice::demo(), true);
        let core = DspiCore::new();
        core.attach(Box::new(t), "LINK".into(), 0);
        core.fetch_all().unwrap();
        log.lock().unwrap().clear();
        core.apply_all_params().unwrap();
        let entries = log.lock().unwrap().clone();
        let write = format!("out {:02X} 0", op::REQ_SET_ALL_PARAMS);
        // -- WP BULKFIX -- The demo lacks the bulk dB fix: the read-back and
        // the unmutes that follow the write are inside the lock too.
        assert_eq!(entries.first().unwrap(), "lock Applying configuration 60");
        assert_eq!(entries[1], write);
        assert_eq!(entries.last().unwrap(), "unlock");
        assert_eq!(entries[2], format!("in {:02X} 0", op::REQ_GET_ALL_PARAMS));
        let unmute = format!("out {:02X} ", op::REQ_SET_OUTPUT_MUTE);
        assert!(
            entries[3..entries.len() - 1]
                .iter()
                .all(|e| e.starts_with(&unmute))
        );
    }

    #[test]
    fn wp_c2_a_refused_lock_sends_nothing() {
        for (refusal, want) in [
            (TransportError::Locked, DspiError::Locked),
            (TransportError::Denied("viewer".into()), DspiError::Denied),
        ] {
            let dev = FakeDevice::demo();
            let remote = dev.remote();
            let (mut t, log) = Spy::new(dev, true);
            t.refuse_lock = Some(refusal);
            let core = DspiCore::new();
            core.attach(Box::new(t), "LINK".into(), 0);
            core.fetch_all().unwrap();
            let before = remote.image();
            core.lock().state.master_volume_db = -50.0;
            assert_eq!(core.apply_all_params(), Err(want));
            assert!(!has_op(&log, "out", op::REQ_SET_ALL_PARAMS));
            assert_eq!(count(&log, "unlock"), 0);
            assert_eq!(remote.image(), before);
            let mut inner = core.lock();
            assert!(!inner.conn().unwrap().echo.own_bulk_pending(Instant::now()));
        }
    }

    #[test]
    fn wp_c2_a_failed_write_still_unlocks() {
        let opts = FakeOptions {
            max_transfer: Some(1024), // refuses the 6136-byte 0xA1
            ..FakeOptions::default()
        };
        let (t, log) = Spy::new(FakeDevice::new(Capture::demo(), opts), true);
        let core = DspiCore::new();
        core.attach(Box::new(t), "LINK".into(), 0);
        // Read over the chunk-free path is single-shot too: 0xA0 also over
        // the limit, so seed the mirror as a connect would have.
        {
            let mut inner = core.lock();
            state::decode_bulk(&demo_bulk(), &mut inner.state).unwrap();
            inner.state.compat = COMPAT_OK;
            inner.conn().unwrap().wire = Some(WireRead::Exact);
        }
        assert!(core.apply_all_params().is_err());
        assert_eq!(count(&log, "unlock"), 1);
    }

    #[test]
    fn wp_c2_fetch_all_after_a_fresh_connect_reads_the_image_once() {
        let dev = FakeDevice::demo();
        let (t, log) = Spy::new(dev, false);
        let core = DspiCore::new();
        core.attach_probed(Box::new(t), "DEMO".into(), 0).unwrap();
        // What C1's attach_probed leaves behind: the mirror decoded from
        // this connect's image, the platform applied, the flag set.
        {
            let mut inner = core.lock();
            let inner = &mut *inner;
            let raw = inner.state.bulk_raw.clone();
            state::decode_bulk(&raw, &mut inner.state).unwrap();
            state::apply_platform(&Capture::demo().platform, &mut inner.state).unwrap();
            inner.state.compat = COMPAT_OK;
            inner.state.core1_mode = 7; // a stale per-item value
            inner.conn.as_mut().unwrap().fresh_image = true;
        }
        let image_start = format!("in {:02X} 0", op::REQ_GET_ALL_PARAMS_CHUNK);
        // The base attach_probed reads the image in probe and again for the
        // mirror; C1's reads it once. Either way fetch_all adds no read.
        let attach_reads = count(&log, &image_start);
        assert!(attach_reads >= 1);
        let platform = format!("in {:02X} 0", op::REQ_GET_PLATFORM);
        let platform_reads = count(&log, &platform);

        core.fetch_all().unwrap();
        assert_eq!(count(&log, &image_start), attach_reads, "no image re-read");
        assert_eq!(
            count(&log, &platform),
            platform_reads,
            "no platform re-read"
        );
        assert!(
            has_op(&log, "in", op::REQ_PRESET_GET_DIR),
            "per-item reads run"
        );
        let s = core.get_state();
        assert_eq!(s.compat, COMPAT_OK);
        assert!(s.bulk_valid);
        assert_eq!(s.master_volume_db, decoded(&demo_bulk()).master_volume_db);
        assert!(s.dac_mute_supported);
        assert_eq!(s.core1_mode, 0, "per-item values reset, then re-read");
        assert!(!core.lock().conn().unwrap().fresh_image, "consumed");

        // A later fetch reads everything again.
        core.fetch_all().unwrap();
        assert_eq!(count(&log, &image_start), attach_reads + 1);
    }

    #[test]
    fn wp_c2_refresh_params_follows_the_device_and_keeps_per_item_values() {
        let (core, remote) = demo_core();
        let names = core.get_state().preset_names;
        core.lock().state.core1_mode = 2; // as a per-item read would set
        // A change at the device: a preset recalled at the unit.
        let mut s = decoded(&remote.image());
        s.master_volume_db = -7.0;
        remote.load_image(state::encode_bulk(&s), nfy::PARAM_SRC_PRESET as u8);
        core.refresh_params().unwrap();
        let st = core.get_state();
        assert_eq!(st.master_volume_db, -7.0);
        assert_eq!(st.core1_mode, 2);
        assert_eq!(st.preset_names, names);
        assert!(st.bulk_valid);
        assert_eq!(st.compat, COMPAT_OK);
    }

    #[test]
    fn wp_c2_fetch_status_parses_the_meter_block() {
        let (core, remote) = demo_core();
        let n = usize::from(core.get_state().num_channels);
        let mut block = vec![0u8; n * 2 + 7];
        block[0..2].copy_from_slice(&0x4000u16.to_le_bytes());
        block[n * 2] = 42;
        block[n * 2 + 1] = 15;
        block[n * 2 + 2..n * 2 + 6].copy_from_slice(&0x8000_0003u32.to_le_bytes());
        block[n * 2 + 6] = 6;
        remote.set_status(block);
        let st = core.fetch_status().unwrap();
        assert_eq!(st.peaks.len(), MAX_CHANNELS);
        assert!((st.peaks[0] - 0.5).abs() < 0.001);
        assert_eq!((st.cpu0, st.cpu1), (42, 15));
        assert_eq!(st.clip_flags, 0x8000_0003);
        assert_eq!(st.active_input_channels, 6);
        assert_eq!(usize::from(st.num_channels), n);

        // A short block is an error, not a panic.
        remote.set_status(vec![1, 2, 3]);
        assert!(matches!(
            core.fetch_status(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c2_parse_status_without_the_input_count() {
        let data = [0xFFu8, 0x7F, 1, 2, 0xFF, 0, 0, 0];
        let st = parse_status(&data, 1).unwrap();
        assert!((st.peaks[0] - 1.0).abs() < 1e-6);
        assert_eq!((st.cpu0, st.cpu1, st.clip_flags), (1, 2, 0xFF));
        assert_eq!(st.active_input_channels, 0);
        assert!(parse_status(&data[..7], 1).is_none());
        assert!(parse_status(&[], 0).is_none());
        // A channel count past MAX_CHANNELS is capped, not trusted.
        assert!(parse_status(&[0u8; 41], 200).is_some());
    }

    #[test]
    fn wp_c2_max_delay_follows_the_platform() {
        let core = DspiCore::new();
        core.lock().state.platform_id = 0;
        assert_eq!(core.max_delay_ms(), 21.0);
        core.lock().state.platform_id = 1;
        assert_eq!(core.max_delay_ms(), 42.0);
    }

    #[test]
    fn wp_c2_clear_clips_works_on_a_read_only_connection() {
        let t = MockTransport::new().data(op::REQ_CLEAR_CLIPS, vec![3u8, 0, 0, 0]);
        let log = t.log_handle();
        let core = DspiCore::new();
        core.attach(Box::new(t), "X".into(), 0);
        // Bare attach: no image read, so not writable.
        assert!(matches!(
            core.lock().writable_conn(),
            Err(DspiError::ReadOnly)
        ));
        core.clear_clips().unwrap();
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.opcode == op::REQ_CLEAR_CLIPS)
        );

        let t = MockTransport::new().reply(op::REQ_CLEAR_CLIPS, Reply::Stall);
        let core = DspiCore::new();
        core.attach(Box::new(t), "X".into(), 0);
        assert!(matches!(
            core.clear_clips(),
            Err(DspiError::Transfer { .. })
        ));
    }

    #[test]
    fn wp_c2_names_are_cut_at_nul_and_31_bytes() {
        assert_eq!(name_from(b"Movie\0junk"), "Movie");
        assert_eq!(name_from(&[b'a'; 40]).len(), 31);
        assert_eq!(name_from(&[]), "");
        assert_eq!(name_from(&[0xFF, b'x']), "\u{FFFD}x");
    }

    #[test]
    fn wp_c2_short_per_item_replies_are_errors() {
        let t = MockTransport::new()
            .data(op::REQ_PRESET_GET_DIR, vec![1u8, 0, 0])
            .data(op::REQ_GET_DAC_HW_MUTE_CONFIG, vec![1u8; 4])
            .data(op::REQ_GET_CTRL_IFACE_STATUS, vec![0u8; 5])
            .data(op::REQ_GET_UART_CONFIG, vec![0u8; 3])
            .reply(op::REQ_PRESET_GET_NAME, Reply::Stall);
        let core = DspiCore::new();
        core.attach(Box::new(t), "X".into(), 0);
        let mut guard = core.lock();
        let inner = &mut *guard;
        let conn = inner.conn.as_mut().unwrap();
        let s = &mut inner.state;
        assert!(read_preset_directory(conn, s).is_err());
        assert!(read_dac_mute(conn, s).is_err());
        assert!(!s.dac_mute_supported);
        assert!(read_ctrl_ifaces(conn, s).is_err());
        assert!(!s.ctrl_iface_supported);
        s.preset_names[2] = "old".into();
        assert!(read_preset_name(conn, s, 2).is_err());
        assert_eq!(s.preset_names[2], "", "an empty slot stalls");
        assert!(matches!(
            read_preset_name(conn, s, MAX_PRESETS as u8),
            Err(DspiError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn wp_c2_get_state_is_a_copy() {
        let (core, _r) = demo_core();
        let mut s = core.get_state();
        s.master_volume_db = 5.0;
        assert_ne!(core.get_state().master_volume_db, 5.0);
    }
}
