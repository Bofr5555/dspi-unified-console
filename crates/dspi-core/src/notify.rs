//! Notifications: the device's change stream applied to the mirror.
//!
//! The reader thread (`dspi_session::Notifications`, started by
//! `attach_probed`) decodes the packets, stamps each with the time it
//! arrived, and wakes the [`NotifyListener`]. The app then calls
//! [`DspiCore::process_notifications`] on its own thread, which:
//!
//! 1. under the lock: drains the queue, records every packet in the monitor
//!    log, classifies each with the connection's echo filter
//!    (`EchoFilter::classify_notification`, at the packet's
//!    `Notification::received` stamp, PLAN §1.2), patches `PARAM_CHANGED`
//!    values into `encode_bulk(state)` and decodes the result once
//!    (PLAN §1.4);
//! 2. then, under the core's lock taken again and held across the read, so no
//!    setter lands between the device's state and the mirror (a hold that
//!    grows to seconds over MIDI and Link): re-reads the whole image
//!    (`connect::refresh_params_locked`, then the preset directory) when the
//!    batch asked for it: a foreign `BULK_INVALIDATED`, a sequence gap or queue overflow
//!    (`Notification::lost`), an offset outside the image, or any
//!    `PARAM_CHANGED` on a read-only connection, whose offsets are another
//!    wire format's (patch 0003).
//!
//! Our own echoes are applied too (the device is authoritative: it may have
//! clamped the value), but they are reported only when they changed the
//! mirror. An own echo of a write sent before our last whole-image write (a
//! recall, an A/B side, an import, a preset load, a factory reset) is not
//! applied at all: the device holds the image's value
//! (`EchoFilter::is_late_own_echo`, WP ECHO1). The legacy v1 master-volume packet (`[0x01, 0, 0, 0, f32]`) the
//! firmware sends ahead of every master-volume `PARAM_CHANGED` is logged and
//! otherwise ignored: its v2 twin carries the change, with a source.
//!
//! No listener is called and no thread joined here (PLAN §1.2 thread rules).
//!
//! Owned by WP C7 (docs/core-merge/PLAN.md).

use std::sync::Arc;
use std::time::Duration;

use dspi_transport::clock::Instant;

use dspi_proto::generated::{self as g, notify as nfy};
use dspi_session::notify::{Event, Notification, Source};
use dspi_transport::TransportKind;

use crate::callbacks::NotifyListener;
use crate::consts::*;
use crate::records::NotifyResult;
use crate::{DspiCore, DspiError, Inner, connect, state};

/// How many times a failed re-read is tried again before the mirror is
/// reported stale ([`NOTIFY_STALE`]). With [`REFRESH_FIRST_DELAY`] doubling,
/// the tries span about 31 s, past a hub's 30 s half-open limit.
const REFRESH_RETRIES: u8 = 5;
/// The wait before the first retry; each later one doubles it.
const REFRESH_FIRST_DELAY: Duration = Duration::from_secs(1);

/// A re-read that failed and is still owed (WP C15, finding C7-4). It lives
/// on the [`crate::Connection`], so it ends with the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RefreshPending {
    /// The re-read state is a preset's: it becomes the baseline.
    rebaseline: bool,
    /// Failed tries since the last change asked for a re-read.
    failures: u8,
    /// When the next try is due. A waker thread calls the
    /// [`NotifyListener`] then; any earlier `process_notifications` skips it.
    next_at: Instant,
}

/// The wait after `failures` failed tries (1 = the first).
fn refresh_delay(failures: u8) -> Duration {
    REFRESH_FIRST_DELAY.saturating_mul(2u32.saturating_pow(u32::from(failures.saturating_sub(1))))
}

/// What the locked phase leaves for the unlocked one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct FollowUp {
    /// Re-read the whole image.
    refresh: bool,
    /// The re-read state is a preset's (a preset load, a factory reset):
    /// it becomes the baseline of `preset_dirty`.
    rebaseline: bool,
    // -- WP PGCORE --
    /// The pin map changed (`NOTIFY_EVT_PIN_MAP_CHANGED`): the pins outside
    /// the image are read again with it.
    pins: bool,
}

// -- WP PGCORE --
/// `NOTIFY_EVT_PIN_MAP_CHANGED` (firmware 1.1.7, `notification_protocol_v2_spec.md`
/// event 0x11, `pin_availability_query_spec.md` 9.2): the device's pin map
/// generation changed (payload: the u32 generation). The pins are read again
/// (PLAN-PG section 1.2, "Notifications"); the coalesced event carries only
/// the newest generation, so any one of them means "re-read".
pub(crate) const EVT_PIN_MAP_CHANGED: u8 = 0x11;

/// `NOTIFY_EVT_INPUT_SOURCE_NAME` and `NOTIFY_EVT_DEVICE_NAME`, which the
/// session's decoder delivers as `Event::Unknown` (payload from byte 4:
/// `[source_id, src, 0, 0, name(32)]`).
pub(crate) const EVT_INPUT_SOURCE_NAME: u8 = nfy::NOTIFY_EVT_INPUT_SOURCE_NAME as u8;
pub(crate) const EVT_DEVICE_NAME: u8 = nfy::NOTIFY_EVT_DEVICE_NAME as u8;

/// Bytes per band (EQ and crossover) in the image.
const BAND_SIZE: usize = 16;
/// PEQ band slots per channel in the image.
const EQ_BANDS: usize = g::wire::WIRE_MAX_BANDS as usize;
/// Crossover bands per channel in the image.
const XOVER_BANDS: usize = g::wire::WIRE_MAX_XOVER_BANDS as usize;

/// Whether the connection's notification reader saw the stream end.
pub(crate) fn stream_ended_locked(inner: &Inner) -> bool {
    // A lost Link session is no departed device: its core reconnects, and the
    // connection and the mirror stay until then (WP C15).
    #[cfg(feature = "link")]
    if crate::link::session_recovers(inner) {
        return false;
    }
    inner
        .conn
        .as_ref()
        .and_then(|c| c.notifications.as_ref())
        .is_some_and(|n| n.is_disconnected())
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Register (`Some`) or clear (`None`) the wake-up listener of the
    /// notification reader. It is called on the reader thread whenever a
    /// packet was queued (and once when the stream ends), and must only
    /// schedule a call to [`DspiCore::process_notifications`].
    ///
    /// Setting or clearing waits for a call in progress on another thread,
    /// so after `None` returns the old listener is not running. Never call
    /// this from inside the listener itself (it would wait for itself).
    pub fn set_notify_listener(&self, listener: Option<Arc<dyn NotifyListener>>) {
        self.listeners.notify.set(listener);
    }

    /// Apply every queued notification to the mirror and say what changed:
    /// `flags` are the `NOTIFY_*` bits (`crate::consts`), `filter_channels`
    /// a bit per wire channel whose PEQ or crossover bands changed.
    ///
    /// - A parameter another host (a knob, the OS volume, UART, MIDI, ...)
    ///   changed is patched into the mirror: `NOTIFY_STATE`, plus
    ///   `NOTIFY_CURVES` when it moves the graph.
    /// - Our own echoes are applied silently, unless the device's value
    ///   differs from the mirror's (then they are reported like a foreign
    ///   change).
    /// - A foreign bulk write, a preset load, lost packets or a change the
    ///   mirror cannot patch re-read the whole image and the preset
    ///   directory: `NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_REFRESHED`. A
    ///   preset load or factory reset also makes the new state the
    ///   baseline of `preset_dirty`, except what was changed on the device
    ///   after the load (an edit made before the notification arrived):
    ///   that stays unsaved. If the re-read fails, the flags say
    ///   only what was patched, and it is tried again: up to 5 times over
    ///   about 31 s, each try announced by a call of the [`NotifyListener`].
    ///   After the last one the result carries [`NOTIFY_STALE`], once; the
    ///   next change that needs a re-read starts over.
    /// - `NOTIFY_DISCONNECTED`: the stream ended because the device went
    ///   away (never after an intentional close). It is reported on every
    ///   call until the connection is closed; the caller should close it
    ///   (`disconnect`, or C1b's hotplug handling). A lost DSPi Link session
    ///   does not count while the core is reconnecting (`link_status`):
    ///   leave the connection alone, and the mirror stays last known.
    ///
    /// Every packet is also recorded in the monitor log
    /// ([`DspiCore::monitor_read`]).
    ///
    /// Without a device, or with nothing queued, it returns an empty
    /// result. Never fails and never panics on device data. Blocks only
    /// for a re-read (one bulk read and the directory read, plus the input
    /// source names after a lost packet).
    pub fn process_notifications(&self) -> NotifyResult {
        self.process_notifications_at(Instant::now())
    }
}

impl DspiCore {
    /// [`DspiCore::process_notifications`] at `now` (tests move the clock).
    pub(crate) fn process_notifications_at(&self, now: Instant) -> NotifyResult {
        let mut result = NotifyResult::default();
        let mut records = Vec::new(); // -- WP P0M --
        let follow = {
            let mut guard = self.lock();
            let mut follow = apply_queued(&mut guard, &mut result, &mut records);
            if let Some(conn) = guard.conn.as_mut()
                && std::mem::take(&mut conn.announce_refresh)
            {
                result.flags |= NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_REFRESHED;
            }
            // -- WP P1EFIX2 -- what a settle consumed since the last call
            if let Some(conn) = guard.conn.as_mut() {
                let settled = std::mem::take(&mut conn.settled);
                result.flags |= settled.flags;
                result.filter_channels |= settled.filter_channels;
            }
            // -- WP P0W --
            let mark = crate::journal::history_mark(&mut guard);
            if mark != guard.history_seen {
                guard.history_seen = mark;
                result.flags |= NOTIFY_HISTORY;
            }
            if let Some(conn) = guard.conn.as_mut()
                && let Some(p) = conn.refresh_pending.as_mut()
            {
                // A new change proves the device is back: try at once and
                // grant the retries anew.
                if follow.refresh {
                    p.failures = 0;
                }
                follow.rebaseline |= p.rebaseline;
                follow.refresh |= now >= p.next_at;
            }
            follow
        };
        // -- WP P0M -- the event log, never written with `Inner` held
        self.events.record_all(records);
        if follow.refresh {
            self.refresh_after_notifications(follow.rebaseline, follow.pins, now, &mut result);
        }
        result
    }
}

// -- WP P1EFIX -- (review p1e-b F1, the Swift test's real cause)
impl DspiCore {
    /// Apply what the device has already said before a whole image is
    /// written (a snapshot recall, an A/B toggle or keep, a whole-device
    /// import, guarded or not): a notification
    /// queued before the image, such as the echo of this console's own
    /// earlier write, would otherwise be applied after it and put an older
    /// value back in the mirror while the device holds the image's. What it
    /// reported (every flag and channel) is reported again by the next
    /// [`DspiCore::process_notifications`], and its event-log records are kept. It is the whole
    /// of `process_notifications`, the re-read included: applying the patches
    /// without the re-read they asked for left a stale value in the mirror
    /// (`wp_p1lk_locked_allows_volume_mute_preset_and_refuses_the_rest`).
    pub(crate) fn settle_notifications(&self) {
        // The whole of process_notifications, the re-read it may need
        // included: a patch the device cannot be trusted on is put right by
        // that re-read, never left in the mirror.
        // -- WP P1EFIX3 -- (review p1efix2 N1) the connection the pump ran on
        let generation = self.lock().conn.as_ref().map(|c| c.generation);
        let r = self.process_notifications_at(Instant::now());
        // -- WP P1EFIX2 -- (review p1efix F1) kept as it was, every flag and
        // channel, for the next call: a settle that only saw the history move
        // claims no refresh, and a NAMES, SIGGEN, CS_AUX, IR_LEARN, STREAMS,
        // INPUT_FORMAT, PRESET or STALE it took still reaches the client
        if let Some(generation) = generation {
            self.keep_settled(generation, r);
        }
    }

    // -- WP P1EFIX3 --
    /// Keep a settle's result for the next pump, on the connection of
    /// `generation` only: after a close and an open in between, the old
    /// connection's flags never reach the new one.
    pub(crate) fn keep_settled(&self, generation: u64, r: NotifyResult) {
        if r == NotifyResult::default() {
            return;
        }
        if let Some(conn) = self
            .lock()
            .conn
            .as_mut()
            .filter(|c| c.generation == generation)
        {
            conn.settled.flags |= r.flags;
            conn.settled.filter_channels |= r.filter_channels;
        }
    }
}

// -- WP ECHO1 --
impl DspiCore {
    /// A whole-image operation begun at `began` (a snapshot recall, an A/B
    /// side, an import, a preset load, a factory reset) is over: the writes
    /// it made after its image belong to it, so an own echo received before
    /// the user's next write is still one older than the image
    /// (`EchoFilter::whole_image_ended`).
    pub(crate) fn whole_image_ended(&self, began: Instant) {
        if let Some(conn) = self.lock().conn.as_mut() {
            conn.echo.whole_image_ended(Instant::now(), began);
        }
    }
}

impl DspiCore {
    /// Whether the open connection's notification stream ended because the
    /// device went away (unplug, closed MIDI port, ended Link session). For
    /// C1b's hotplug handling; false with no device or no reader. An
    /// intentional close never sets it (the reader is stopped before the
    /// transport is dropped, and the connection is gone from the core).
    #[cfg(test)]
    pub(crate) fn notification_stream_ended(&self) -> bool {
        stream_ended_locked(&self.lock())
    }

    /// The unlocked phase: re-read the image (C2's path), then the preset
    /// directory, and rebaseline if a preset was loaded (keeping what
    /// changed after the load unsaved, WP CG33).
    fn refresh_after_notifications(
        &self,
        rebaseline: bool,
        pins: bool, // -- WP PGCORE --
        now: Instant,
        result: &mut NotifyResult,
    ) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        // -- WP CG33 -- What the re-read must not count as saved.
        let before = rebaseline.then(|| (inner.baseline.clone(), inner.state.bulk_raw.clone()));
        if let Err(e) = connect::refresh_params_locked(inner) {
            log::warn!("re-reading the device after a notification failed: {e}");
            let wake = inner
                .conn
                .as_mut()
                .and_then(|conn| refresh_failed(conn, &e, rebaseline, now, result));
            drop(guard);
            if let Some(delay) = wake {
                self.wake_notify_after(delay);
            }
            return;
        }
        if let Some(conn) = inner.conn.as_mut() {
            conn.refresh_pending = None;
            // A failure must not undo the re-read: the directory keeps its
            // last value.
            if let Err(e) = connect::read_preset_directory(conn, &mut inner.state) {
                log::debug!("preset directory not re-read: {e}");
            }
            // -- WP CG13 --
            if std::mem::take(&mut conn.source_names_stale) {
                crate::info::read_source_names(conn);
                // -- WP QD4 --
                crate::info::read_device_name(conn);
            }
            // -- WP PGCORE -- the pins the image does not hold: the DAC mute
            // pin and the control interfaces' (a failed read keeps the last)
            if pins {
                if inner.state.dac_mute_supported
                    && let Err(e) = connect::read_dac_mute(conn, &mut inner.state)
                {
                    log::debug!("DAC mute pin not re-read: {e}");
                }
                if let Err(e) = connect::read_ctrl_ifaces(conn, &mut inner.state) {
                    log::debug!("control interface pins not re-read: {e}");
                }
            }
        }
        if let Some((old, mirror)) = before {
            crate::presets::rebaseline_after_load_locked(inner, old.as_deref(), &mirror);
        }
        result.flags |= NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_REFRESHED;
    }

    /// Call the [`NotifyListener`] once after `delay`, so the app comes back
    /// for a due retry even if the device stays silent. The thread holds the
    /// listeners, not the core, and ends by itself.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    fn wake_notify_after(&self, delay: Duration) {
        let listeners = Arc::clone(&self.listeners);
        let spawned = std::thread::Builder::new()
            .name("dspi-refresh-retry".into())
            .spawn(move || {
                dspi_transport::clock::sleep(delay);
                listeners.notify.call(|l| l.on_notifications());
            });
        if let Err(e) = spawned {
            log::warn!("cannot schedule a re-read retry: {e}");
        }
    }

    /// No threads on the web (WP WB2): the caller asks
    /// [`DspiCore::notify_retry_due_in`] when to come back instead.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn wake_notify_after(&self, delay: Duration) {
        log::debug!("a re-read retry is due in {delay:?}");
    }
}

// -- WP WB2 --
/// The caller-driven side of notifications, for a build without threads
/// (`wasm32-unknown-unknown`), where the reader is
/// `dspi_session::Notifications::caller_driven` and nothing wakes the app.
/// Not exported over UniFFI or the C ABI; the web binding (WB3) calls these.
impl DspiCore {
    /// Hand in a notification packet the caller read itself (a page's own
    /// endpoint or socket loop), with the link's origin, then call
    /// [`DspiCore::process_notifications`] as after a wake. On the web a
    /// connection without a notification source gets a reader for this on
    /// the first push.
    ///
    /// [`DspiError::Disconnected`] without a device; [`DspiError::Unsupported`]
    /// when the connection's reader runs on its own thread (it reads its own
    /// source), or natively when it has none.
    pub fn push_notification(&self, packet: Vec<u8>, origin: Option<u16>) -> crate::Result<()> {
        let mut inner = self.lock();
        let conn = inner.conn.as_mut().ok_or(DspiError::Disconnected)?;
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if conn.notifications.is_none() {
            conn.notifications = Some(dspi_session::Notifications::caller_driven(None));
        }
        match conn.notifications.as_ref() {
            Some(reader) if reader.push(packet, origin) => Ok(()),
            _ => Err(DspiError::Unsupported),
        }
    }

    /// How long until a failed re-read is due to be tried again: call
    /// [`DspiCore::process_notifications`] then. Zero when it is overdue,
    /// `None` with nothing owed. Natively a thread wakes the
    /// [`NotifyListener`] for this; on the web the caller sets a timer.
    pub fn notify_retry_due_in(&self) -> Option<Duration> {
        let inner = self.lock();
        let pending = inner.conn.as_ref()?.refresh_pending?;
        Some(pending.next_at.saturating_duration_since(Instant::now()))
    }
}

/// Note a failed re-read on `conn`. Returns the delay of a new retry that
/// needs a wake-up: none if one is already armed, none when giving up.
fn refresh_failed(
    conn: &mut crate::Connection,
    e: &DspiError,
    rebaseline: bool,
    now: Instant,
    result: &mut NotifyResult,
) -> Option<Duration> {
    let previous = conn.refresh_pending.take();
    // Gone: the reconnect path reads everything. Refused: it would be again.
    if matches!(e, DspiError::Disconnected | DspiError::Denied) {
        return None;
    }
    let failures = previous.map_or(0, |p| p.failures).saturating_add(1);
    if failures > REFRESH_RETRIES {
        log::warn!("giving up re-reading the device; the mirror may be stale");
        result.flags |= NOTIFY_STALE;
        return None;
    }
    let delay = refresh_delay(failures);
    // A waker from an earlier failure is still sleeping: it covers this try.
    if let Some(p) = previous.filter(|p| p.next_at > now) {
        conn.refresh_pending = Some(RefreshPending {
            rebaseline,
            failures,
            ..p
        });
        return None;
    }
    conn.refresh_pending = Some(RefreshPending {
        rebaseline,
        failures,
        next_at: now.checked_add(delay).unwrap_or(now),
    });
    Some(delay)
}

/// The locked phase of `process_notifications`: drain, log, classify and
/// patch. Returns whether a re-read is needed.
/// -- WP P0M -- `records`: the event log's, written once `Inner` is released.
fn apply_queued(
    inner: &mut Inner,
    r: &mut NotifyResult,
    records: &mut Vec<dspi_session::events::EventRecord>,
) -> FollowUp {
    let mut follow = FollowUp::default();
    let ended = stream_ended_locked(inner);
    let Some(conn) = inner.conn.as_mut() else {
        return follow;
    };
    let Some(reader) = conn.notifications.as_ref() else {
        return follow;
    };
    let notes = reader.drain();
    let kind = conn.caps.kind;
    let session = conn.session_id;
    // Patch only an image in this build's own layout; another wire format's
    // offsets mean something else (§1.5).
    let patchable = conn.writable() && inner.state.bulk_valid;
    // An unmappable image (no `wire`) cannot be re-read either.
    let rereadable = conn.wire.is_some();
    // Started lazily from the state, not from `bulk_raw`: decoding a stale
    // image would undo setters that changed only the record.
    let mut image: Option<Vec<u8>> = None;

    for note in &notes {
        inner.monitor.record(note);
        let own = conn.echo.classify_notification(kind, session, note);
        // -- WP CG23 --
        inner.notify_tap.record(note, own);
        // -- WP P5SES -- what a running measurement must judge (S5)
        // -- WP P5FOLLOW -- an own echo that leaves the mirror as it is
        let unchanged = own
            && patchable
            && inner.measure_tap.is_on()
            && match &note.event {
                Event::ParamChanged { offset, bytes, .. } => {
                    let img = image.get_or_insert_with(|| state::encode_bulk(&inner.state));
                    let at = usize::from(*offset);
                    at.checked_add(bytes.len())
                        .and_then(|end| img.get(at..end))
                        .is_some_and(|held| held == bytes.as_slice())
                }
                _ => false,
            };
        crate::measure::note_notification_echo(&mut inner.measure_tap, note, own, unchanged);
        // -- WP P0M --
        inner
            .events
            .notification(note, own, conn.ext.notify_features, records);
        if note.lost {
            follow.refresh = true;
            // -- WP CG13 --
            conn.source_names_stale = true;
        }
        // -- WP CG21 -- The fill target follows the source, the rate and the preset
        if note.lost
            || matches!(
                note.event,
                Event::BulkInvalidated { .. }
                    | Event::PresetLoaded { .. }
                    | Event::InputFormat { .. }
            )
        {
            conn.ring_target.invalidate();
        }
        match &note.event {
            Event::ParamChanged {
                offset,
                bytes,
                source,
            } => {
                // -- WP P0W -- who, for undo's foreign-change rule
                if !own {
                    crate::journal::note_foreign(conn, *source, note.origin);
                }
                // -- WP ECHO1 -- (review p1efix F2) the echo of a write of ours
                // sent before our last whole image: older than the image the
                // device now holds, so it is not applied (nor re-read for)
                if own && conn.echo.is_late_own_echo(kind, session, note) {
                    continue;
                }
                if !patchable {
                    follow.refresh = true;
                    continue;
                }
                let img = image.get_or_insert_with(|| state::encode_bulk(&inner.state));
                let at = usize::from(*offset);
                match patch(img, at, bytes) {
                    Some(changed) if !own || changed => {
                        r.flags |= NOTIFY_STATE | graph_effect(at, &mut r.filter_channels);
                    }
                    Some(_) => {}
                    None => follow.refresh = true,
                }
            }
            Event::BulkInvalidated { source } => {
                if !own {
                    crate::journal::note_foreign(conn, *source, note.origin); // -- WP P0W --
                    follow.refresh = true;
                    if matches!(source, Source::Preset | Source::Factory) {
                        follow.rebaseline = true;
                    }
                }
            }
            Event::PresetLoaded { slot } => {
                inner.state.active_preset_slot = *slot;
                r.flags |= NOTIFY_STATE | NOTIFY_PRESET;
            }
            Event::InputFormat { .. } => r.flags |= NOTIFY_STATE | NOTIFY_INPUT_FORMAT,
            Event::SiggenState { .. } => r.flags |= NOTIFY_SIGGEN,
            // Our own aux writes are already in the control-surface state
            // (WP C14 re-reads what it writes).
            Event::CsAux {
                slot,
                state,
                level_q8,
                source,
            } => {
                if !is_own(kind, source.raw(), note, session) {
                    inner.cs.note_aux(usize::from(*slot), *state, *level_q8);
                    r.flags |= NOTIFY_CS_AUX;
                }
            }
            Event::IrLearn {
                state,
                protocol,
                code,
            } => {
                inner.cs.note_learn(*state, *protocol, *code);
                r.flags |= NOTIFY_IR_LEARN;
            }
            Event::Unknown { id, bytes }
                if *id == EVT_INPUT_SOURCE_NAME || *id == EVT_DEVICE_NAME =>
            {
                let src = bytes.get(1).copied().unwrap_or(0);
                // -- WP CG13 --
                if *id == EVT_INPUT_SOURCE_NAME {
                    crate::info::note_source_name_event(conn, bytes);
                } else {
                    // -- WP QD4 --
                    crate::info::note_device_name_event(conn, bytes);
                }
                if !is_own(kind, src, note, session) {
                    r.flags |= NOTIFY_NAMES;
                }
            }
            // -- WP CG5 --
            Event::Unknown { id, bytes } if *id == crate::rta::EVT_RTA_CONFIG => {
                let src = bytes
                    .get(crate::rta::RTA_CONFIG_SOURCE_AT)
                    .copied()
                    .unwrap_or(0);
                let own = is_own(kind, src, note, session);
                inner.rta.note_config(bytes, own, note.origin.unwrap_or(0));
            }
            // -- WP CG25 -- Stream states. The mirror follows ADAT out, as
            // the Mac does; the rest are re-read by the client.
            Event::AdatState { enabled, pin, .. } => {
                // A started image would undo the change when it is decoded
                // below: apply it first.
                if let Some(img) = image.take()
                    && let Err(e) = state::decode_bulk(&img, &mut inner.state)
                {
                    log::warn!("a parameter notification could not be applied: {e}");
                    follow.refresh = true;
                }
                let s = &mut inner.state;
                let pin = if *pin != 0 { *pin } else { s.adat_out_pin };
                if (s.adat_out_enabled, s.adat_out_pin) != (*enabled, pin) {
                    (s.adat_out_enabled, s.adat_out_pin) = (*enabled, pin);
                    r.flags |= NOTIFY_STATE;
                }
                r.flags |= NOTIFY_STREAMS;
            }
            Event::I2sSlaveState { .. } | Event::AdatInputState { .. } => {
                r.flags |= NOTIFY_STREAMS;
            }
            // -- WP FWX -- A clip event, read only from a device announcing it
            // (EXT_NOTIFY_FEATURES bit 1); `take_clip_mask` hands it on.
            Event::Unknown { id, bytes } if *id == dspi_proto::ext::NOTIFY_EVT_EXT_CLIP => {
                crate::ext_diag::note_clip(conn, *id, bytes);
            }
            // -- WP PGCORE -- the pin map changed: re-read the pins (the
            // image holds most of them, the rest follow it)
            Event::Unknown { id, .. } if *id == EVT_PIN_MAP_CHANGED => {
                follow.refresh = true;
                follow.pins = true;
            }
            // The v1 master volume is deduplicated against its v2 twin (the
            // `PARAM_CHANGED` that follows it); the link states and unknown
            // events are only logged.
            Event::MasterVolume(_) | Event::Idle | Event::Unknown { .. } => {}
        }
    }

    if let Some(img) = image {
        // `encode_bulk` always yields an image of the right size, so this
        // fails only if the state's own header is not V32 (it then cannot
        // be patched: re-read instead).
        if let Err(e) = state::decode_bulk(&img, &mut inner.state) {
            log::warn!("a parameter notification could not be applied: {e}");
            follow.refresh = true;
        }
    }
    if ended {
        r.flags |= NOTIFY_DISCONNECTED;
        follow = FollowUp::default();
    }
    follow.refresh &= rereadable;
    follow.rebaseline &= follow.refresh;
    follow.pins &= follow.refresh; // -- WP PGCORE --
    follow
}

/// The echo rule for events the echo filter does not classify (they carry
/// their source elsewhere): never a bulk echo.
fn is_own(kind: TransportKind, source: u8, note: &Notification, session: Option<u16>) -> bool {
    dspi_transport::is_own_write(kind, source, note.origin, session, false)
}

/// Copy `bytes` into `img` at `offset`: `Some(changed)`, or `None` when the
/// range touches the header or runs off the image (the device and this
/// build disagree on the layout: re-read).
fn patch(img: &mut [u8], offset: usize, bytes: &[u8]) -> Option<bool> {
    if offset < g::OFF_GLOBAL {
        return None;
    }
    let end = offset.checked_add(bytes.len())?;
    let dst = img.get_mut(offset..end)?;
    let changed = dst != bytes;
    dst.copy_from_slice(bytes);
    // -- WP CG31 -- A value the image holds twice changes in both places
    let twin = dspi_proto::wire::sync_twins(img, offset, bytes.len());
    Some(changed || twin)
}

/// `NOTIFY_CURVES` when a change at `offset` moves the graph (bands, global,
/// outputs, preamps), else 0; a band change also marks its wire channel in
/// `filter_channels`.
fn graph_effect(offset: usize, filter_channels: &mut u32) -> u32 {
    let in_section = |base: usize, len: usize| offset.checked_sub(base).filter(|r| *r < len);
    let channel_bit = |rel: usize, bands: usize| {
        rel.checked_div(BAND_SIZE.saturating_mul(bands))
            .and_then(|ch| u32::try_from(ch).ok())
            .and_then(|ch| 1u32.checked_shl(ch))
            .unwrap_or(0)
    };
    if let Some(rel) = in_section(g::OFF_EQ, g::LEN_EQ) {
        *filter_channels |= channel_bit(rel, EQ_BANDS);
        NOTIFY_CURVES
    } else if let Some(rel) = in_section(g::OFF_CROSSOVERS, g::LEN_CROSSOVERS) {
        *filter_channels |= channel_bit(rel, XOVER_BANDS);
        NOTIFY_CURVES
    } else if in_section(g::OFF_GLOBAL, g::LEN_GLOBAL).is_some()
        || in_section(g::OFF_OUTPUTS, g::LEN_OUTPUTS).is_some()
        || in_section(g::OFF_PREAMP, g::LEN_PREAMP).is_some()
    {
        NOTIFY_CURVES
    } else {
        0
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c7 {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};

    use dspi_proto::generated::opcodes as op;
    use dspi_transport::{
        DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
        TransportError,
    };

    use super::*;

    const SRC_HOST: u8 = nfy::PARAM_SRC_HOST_SET as u8;
    const SRC_BULK: u8 = nfy::PARAM_SRC_BULK_SET as u8;
    const SRC_PRESET: u8 = nfy::PARAM_SRC_PRESET as u8;
    const SRC_GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;
    const SRC_UAC1: u8 = nfy::PARAM_SRC_UAC1 as u8;

    /// A core on the demo device, with a remote to act as the device.
    fn demo() -> (Arc<DspiCore>, FakeRemote) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        settle(&core);
        (core, remote)
    }

    /// The demo device whose image reads fail while the flag is up.
    struct Flaky {
        inner: FakeDevice,
        failing: Arc<AtomicBool>,
    }

    impl Transport for Flaky {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            if opcode == op::REQ_GET_ALL_PARAMS && self.failing.load(Ordering::SeqCst) {
                return Err(TransportError::TimedOut { opcode, retries: 0 });
            }
            self.inner.control_in(opcode, value, len)
        }
        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            if matches!(
                opcode,
                op::REQ_GET_ALL_PARAMS | op::REQ_GET_ALL_PARAMS_CHUNK
            ) && self.failing.load(Ordering::SeqCst)
            {
                return Err(TransportError::TimedOut { opcode, retries: 0 });
            }
            self.inner.control_in_upto(opcode, value, len)
        }
        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            self.inner.control_out(opcode, value, data)
        }
        fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
            self.inner.descriptor()
        }
        fn notifications(&self) -> Option<Box<dyn dspi_transport::NotificationSource>> {
            self.inner.notifications()
        }
        fn caps(&self) -> dspi_transport::TransportCaps {
            self.inner.caps()
        }
    }

    fn flaky() -> (Arc<DspiCore>, FakeRemote, Arc<AtomicBool>) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let failing = Arc::new(AtomicBool::new(false));
        let core = DspiCore::new();
        let t = Flaky {
            inner: dev,
            failing: Arc::clone(&failing),
        };
        core.attach_probed(Box::new(t), "DEMO".into(), 0).unwrap();
        settle(&core);
        (core, remote, failing)
    }

    /// A foreign write whose re-read fails: until it has been noted.
    fn failed_refresh(core: &DspiCore, remote: &FakeRemote, source: u8, volume: f32) {
        let mut img = remote.image();
        img[g::OFF_MASTER_VOLUME..][..4].copy_from_slice(&volume.to_le_bytes());
        remote.load_image(img, source);
        let start = Instant::now();
        while core.lock().conn().unwrap().refresh_pending.is_none() {
            core.process_notifications();
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn wp_c15_a_failed_re_read_is_retried_and_rebaselines() {
        let (core, remote, failing) = flaky();
        failing.store(true, Ordering::SeqCst);
        failed_refresh(&core, &remote, SRC_PRESET, -44.0);
        assert_ne!(core.get_state().master_volume_db, -44.0);
        // Not due yet: nothing happens.
        let now = Instant::now();
        assert_eq!(core.process_notifications_at(now), NotifyResult::default());

        failing.store(false, Ordering::SeqCst);
        let later = now + Duration::from_secs(2);
        let r = core.process_notifications_at(later);
        assert_ne!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
        assert_eq!(core.get_state().master_volume_db, -44.0);
        let inner = core.lock();
        assert!(inner.conn.as_ref().unwrap().refresh_pending.is_none());
        assert!(inner.baseline.is_some(), "the preset's rebaseline survived");
    }

    #[test]
    fn wp_c15_retries_are_limited_and_end_in_stale_once() {
        let (core, remote, failing) = flaky();
        failing.store(true, Ordering::SeqCst);
        failed_refresh(&core, &remote, SRC_BULK, -45.0);
        let mut at = Instant::now();
        let mut tries = 0;
        let mut stale = 0;
        for _ in 0..12 {
            at += Duration::from_secs(40);
            let r = core.process_notifications_at(at);
            stale += u32::from(r.flags & NOTIFY_STALE != 0);
            if core.lock().conn().unwrap().refresh_pending.is_some() {
                tries += 1;
            }
        }
        assert_eq!(tries, REFRESH_RETRIES - 1, "the first failure counts");
        assert_eq!(stale, 1);

        // The next change starts over, and it works.
        failing.store(false, Ordering::SeqCst);
        remote.load_image(remote.image(), SRC_BULK);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
    }

    // -- WP WB2 --
    #[test]
    fn wb2_the_retry_deadline_is_readable_for_a_caller_driven_timer() {
        let (core, remote, failing) = flaky();
        assert_eq!(core.notify_retry_due_in(), None, "nothing owed");
        failing.store(true, Ordering::SeqCst);
        failed_refresh(&core, &remote, SRC_BULK, -43.0);
        let due = core.notify_retry_due_in().expect("a retry is owed");
        assert!(due <= REFRESH_FIRST_DELAY, "{due:?}");
        failing.store(false, Ordering::SeqCst);
        core.process_notifications_at(Instant::now() + Duration::from_secs(2));
        assert_eq!(core.notify_retry_due_in(), None, "the retry worked");
    }

    // -- WP WB2 --
    #[test]
    fn wb2_a_push_needs_a_device_and_a_caller_driven_reader() {
        let core = DspiCore::new();
        assert!(matches!(
            core.push_notification(vec![0], None),
            Err(DspiError::Disconnected)
        ));
        let (core, _remote) = demo();
        // Natively the demo's reader has its own thread.
        assert!(matches!(
            core.push_notification(vec![0], None),
            Err(DspiError::Unsupported)
        ));
    }

    // -- WP WB2 --
    #[test]
    fn wb2_a_pushed_packet_is_applied_like_a_read_one() {
        let (core, _remote) = demo();
        core.lock().conn.as_mut().unwrap().notifications =
            Some(dspi_session::Notifications::caller_driven(None));
        let mut p = ev(nfy::NOTIFY_EVT_PARAM_CHANGED, &[]);
        p[3] = 1;
        p.extend((g::OFF_MASTER_VOLUME as u16).to_le_bytes());
        p.extend(4u16.to_le_bytes());
        p.extend([SRC_GPIO, 0, 0, 0]);
        p.extend((-31.5f32).to_le_bytes());
        core.push_notification(p, None).unwrap();
        let r = core.process_notifications();
        assert_ne!(r.flags & NOTIFY_STATE, 0, "{r:?}");
        assert_eq!(core.get_state().master_volume_db, -31.5);
    }

    #[test]
    fn wp_c15_the_retry_wakes_the_listener_without_a_new_event() {
        struct Wake(AtomicBool);
        impl NotifyListener for Wake {
            fn on_notifications(&self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (core, remote, failing) = flaky();
        failing.store(true, Ordering::SeqCst);
        failed_refresh(&core, &remote, SRC_BULK, -46.0);
        settle(&core);
        let wake = Arc::new(Wake(AtomicBool::new(false)));
        core.set_notify_listener(Some(wake.clone()));
        let start = Instant::now();
        while !wake.0.load(Ordering::SeqCst) {
            assert!(start.elapsed() < Duration::from_secs(4), "no wake-up");
            std::thread::sleep(Duration::from_millis(10));
        }
        core.set_notify_listener(None);
    }

    #[test]
    fn wp_c15_a_gone_device_is_not_retried() {
        let mut pending = crate::Connection::new(Box::new(FakeDevice::demo()), "D".into(), 0);
        let mut r = NotifyResult::default();
        let now = Instant::now();
        assert_eq!(
            refresh_failed(&mut pending, &DspiError::Disconnected, false, now, &mut r),
            None
        );
        assert!(pending.refresh_pending.is_none());
        let e = DspiError::Transfer { msg: "x".into() };
        assert_eq!(
            refresh_failed(&mut pending, &e, false, now, &mut r),
            Some(REFRESH_FIRST_DELAY)
        );
        assert_eq!(refresh_delay(1), REFRESH_FIRST_DELAY);
        assert_eq!(refresh_delay(5), REFRESH_FIRST_DELAY * 16);
    }

    /// Process until `f` holds for the accumulated result (the reader
    /// thread delivers asynchronously).
    fn process_until(core: &DspiCore, mut f: impl FnMut(&NotifyResult) -> bool) -> NotifyResult {
        let start = Instant::now();
        let mut acc = NotifyResult::default();
        loop {
            let r = core.process_notifications();
            acc.flags |= r.flags;
            acc.filter_channels |= r.filter_channels;
            if f(&acc) {
                return acc;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out; got {acc:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Process until the reader has had nothing to deliver for a while.
    fn settle(core: &DspiCore) -> NotifyResult {
        let mut acc = NotifyResult::default();
        let mut quiet = 0;
        while quiet < 10 {
            std::thread::sleep(Duration::from_millis(10));
            let r = core.process_notifications();
            if r == NotifyResult::default() {
                quiet += 1;
            } else {
                quiet = 0;
            }
            acc.flags |= r.flags;
            acc.filter_channels |= r.filter_channels;
        }
        acc
    }

    fn write_raw(core: &DspiCore, opcode: u8, value: u16, data: &[u8]) {
        core.lock()
            .writable_conn()
            .unwrap()
            .set(opcode, value, data)
            .unwrap();
    }

    /// A v2 event packet (the fake renumbers it).
    fn ev(id: u16, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![2, id as u8, 0, 0];
        p.extend_from_slice(payload);
        p
    }

    fn name_packet(evt: u8, id: u8, src: u8, name: &[u8]) -> Vec<u8> {
        let mut p = vec![2, evt, 0, 0, id, src, 0, 0];
        let mut field = [0u8; 32];
        field[..name.len()].copy_from_slice(name);
        p.extend(field);
        p
    }

    #[test]
    fn wp_c7_without_a_device_nothing_happens() {
        let core = DspiCore::new();
        assert_eq!(core.process_notifications(), NotifyResult::default());
        assert!(!core.notification_stream_ended());
    }

    #[test]
    fn wp_c7_a_foreign_change_is_patched_into_the_mirror() {
        let (core, remote) = demo();
        core.lock().state.filters[0][0].gain = 4.5; // a local edit, not in bulk_raw
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-12.0f32).to_le_bytes(), SRC_UAC1));
        let r = process_until(&core, |r| r.flags & NOTIFY_STATE != 0);
        assert_eq!(
            r.flags & NOTIFY_CURVES,
            0,
            "the volume does not move the graph"
        );
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0);
        let s = core.get_state();
        assert_eq!(s.master_volume_db, -12.0);
        assert_eq!(s.filters[0][0].gain, 4.5, "local edits survive a patch");
        assert_eq!(
            &s.bulk_raw[g::OFF_MASTER_VOLUME..][..4],
            &(-12.0f32).to_le_bytes()
        );
    }

    #[test]
    fn wp_c7_a_band_change_marks_its_channel() {
        let (core, remote) = demo();
        let mut band = [0u8; 16];
        band[0] = 1;
        band[4..8].copy_from_slice(&1000.0f32.to_le_bytes());
        band[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        band[12..16].copy_from_slice(&3.0f32.to_le_bytes());
        let off = g::OFF_EQ + (3 * EQ_BANDS + 2) * BAND_SIZE;
        assert!(remote.poke(off, &band, SRC_GPIO));
        let r = process_until(&core, |r| r.filter_channels != 0);
        assert_eq!(r.filter_channels, 1 << 3);
        assert_eq!(r.flags & NOTIFY_CURVES, NOTIFY_CURVES);
        let s = core.get_state();
        assert_eq!(s.filters[3][2].freq, 1000.0);
        assert_eq!(s.filters[3][2].gain, 3.0);
    }

    #[test]
    fn wp_c7_our_own_echo_and_its_v1_twin_are_silent() {
        let (core, _remote) = demo();
        // What a setter does: send, then update the record.
        write_raw(
            &core,
            op::REQ_SET_MASTER_VOLUME,
            0,
            &(-7.0f32).to_le_bytes(),
        );
        core.lock().state.master_volume_db = -7.0;
        let r = settle(&core);
        assert_eq!(r, NotifyResult::default(), "own echo (v1 + v2) is silent");
        assert_eq!(core.get_state().master_volume_db, -7.0);
        let log = core.monitor_read(0);
        assert!(
            log.entries
                .iter()
                .any(|e| e.text.starts_with("v1.MasterVolume")),
            "{log:?}"
        );
        assert!(
            log.entries
                .iter()
                .any(|e| e.text.contains("[HOST    ] master_volume.master_volume_db")),
            "{log:?}"
        );
    }

    #[test]
    fn wp_c7_an_echo_the_device_changed_is_applied_and_reported() {
        let (core, _remote) = demo();
        write_raw(
            &core,
            op::REQ_SET_MASTER_VOLUME,
            0,
            &(-9.0f32).to_le_bytes(),
        );
        // The record says something else (as if the device had clamped it).
        core.lock().state.master_volume_db = -3.0;
        let r = process_until(&core, |r| r.flags & NOTIFY_STATE != 0);
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0);
        assert_eq!(core.get_state().master_volume_db, -9.0, "the device wins");
    }

    #[test]
    fn wp_c7_a_lone_v1_volume_packet_changes_nothing() {
        let (core, remote) = demo();
        let before = core.get_state().master_volume_db;
        let mut v1 = vec![nfy::NOTIFY_EVT_MASTER_VOLUME as u8, 0, 0, 0];
        v1.extend((-33.0f32).to_le_bytes());
        remote.push(v1);
        let start = Instant::now();
        while core.monitor_read(0).entries.is_empty() {
            assert_eq!(core.process_notifications(), NotifyResult::default());
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(core.get_state().master_volume_db, before);
    }

    #[test]
    fn wp_c7_a_foreign_bulk_write_re_reads_and_a_preset_rebaselines() {
        let (core, remote) = demo();
        let mut img = remote.image();
        img[g::OFF_MASTER_VOLUME..][..4].copy_from_slice(&(-40.0f32).to_le_bytes());
        remote.load_image(img.clone(), SRC_BULK);
        let r = process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        assert_eq!(r.flags & NOTIFY_STATE, NOTIFY_STATE);
        assert_eq!(core.get_state().master_volume_db, -40.0);
        assert!(
            core.lock().baseline.is_none(),
            "a bulk write is not a preset"
        );

        img[g::OFF_MASTER_VOLUME..][..4].copy_from_slice(&(-41.0f32).to_le_bytes());
        remote.load_image(img, SRC_PRESET);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        let inner = core.lock();
        assert_eq!(inner.state.master_volume_db, -41.0);
        assert_eq!(
            inner.baseline.as_deref(),
            Some(inner.state.bulk_raw.as_slice())
        );
    }

    #[test]
    fn wp_c7_our_own_bulk_write_is_not_re_read() {
        let (core, remote) = demo();
        core.apply_all_params().unwrap();
        let r = settle(&core);
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
        // The next one is someone else's.
        remote.load_image(remote.image(), SRC_BULK);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
    }

    /// Firmware that tags bulk applies (`EXT_NOTIFY_FEATURES`,
    /// `NOTIFY_FEAT_BULK_SOURCE`): the probe switches the filter over.
    fn tagged() -> (Arc<DspiCore>, FakeRemote) {
        let dev = FakeDevice::new(
            dspi_transport::fake::Capture::demo(),
            dspi_transport::fake::FakeOptions {
                extensions: true,
                bulk_source: true,
                ..Default::default()
            },
        );
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        settle(&core);
        (core, remote)
    }

    #[test]
    fn wp_b2_the_probe_chooses_the_echo_rule() {
        let (core, _) = demo();
        assert!(!core.lock().conn().unwrap().echo.bulk_source());
        let (core, _) = tagged();
        assert!(core.lock().conn().unwrap().echo.bulk_source());
    }

    #[test]
    fn wp_b2_tagged_our_own_bulk_write_is_not_re_read() {
        let (core, remote) = tagged();
        core.apply_all_params().unwrap();
        let r = settle(&core);
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
        // The same path with nothing of ours outstanding: another app.
        remote.load_image(remote.image(), nfy::PARAM_SRC_HOST_SET as u8);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
    }

    #[test]
    fn wp_b2_tagged_any_other_source_re_reads() {
        for source in [
            nfy::PARAM_SRC_UNKNOWN,
            nfy::PARAM_SRC_BULK_SET,
            nfy::PARAM_SRC_MIDI,
        ] {
            let (core, remote) = tagged();
            core.lock()
                .conn()
                .unwrap()
                .echo
                .bulk_write_started(Instant::now());
            remote.load_image(remote.image(), source as u8);
            process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        }
    }

    /// Without the feature, a `HOST_SET` invalidation (which older firmware
    /// never sends) answers no bulk write of ours.
    #[test]
    fn wp_b2_the_window_rule_only_claims_bulk_set() {
        let (core, remote) = demo();
        core.lock()
            .conn()
            .unwrap()
            .echo
            .bulk_write_started(Instant::now());
        remote.load_image(remote.image(), nfy::PARAM_SRC_HOST_SET as u8);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
    }

    #[test]
    fn wp_c7_a_late_drain_still_claims_our_bulk_echo() {
        // Classification uses the reader's stamp, not the drain time.
        let (core, _remote) = demo();
        core.apply_all_params().unwrap();
        std::thread::sleep(dspi_session::notify::EchoFilter::WINDOW + Duration::from_millis(300));
        let r = settle(&core);
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
    }

    #[test]
    fn wp_c7_preset_loaded_sets_the_slot() {
        let (core, remote) = demo();
        remote.push(ev(nfy::NOTIFY_EVT_PRESET_LOADED, &[5, 0, 0, 0]));
        let r = process_until(&core, |r| r.flags & NOTIFY_PRESET != 0);
        assert_eq!(r.flags & NOTIFY_STATE, NOTIFY_STATE);
        assert_eq!(core.get_state().active_preset_slot, 5);
    }

    #[test]
    fn wp_c7_events_map_to_their_flags() {
        let (core, remote) = demo();
        remote.push(ev(nfy::NOTIFY_EVT_INPUT_FORMAT, &[4, 0, 0, 0]));
        process_until(&core, |r| r.flags & NOTIFY_INPUT_FORMAT != 0);
        remote.push(ev(nfy::NOTIFY_EVT_SIGGEN_STATE, &[2, 1, 0, 0xFF]));
        let r = process_until(&core, |r| r.flags & NOTIFY_SIGGEN != 0);
        assert_eq!(
            r.flags & NOTIFY_STATE,
            0,
            "the generator is not a parameter"
        );
        remote.push(ev(nfy::NOTIFY_EVT_CS_IR_LEARN, &[2, 1, 0, 0, 1, 2, 3, 4]));
        process_until(&core, |r| r.flags & NOTIFY_IR_LEARN != 0);
        remote.push(ev(nfy::NOTIFY_EVT_CS_AUX, &[1, 1, 0, 0x40, SRC_GPIO]));
        process_until(&core, |r| r.flags & NOTIFY_CS_AUX != 0);
        // Our own aux write is not news.
        remote.push(ev(nfy::NOTIFY_EVT_CS_AUX, &[1, 0, 0, 0, SRC_HOST]));
        assert_eq!(settle(&core).flags & NOTIFY_CS_AUX, 0);
    }

    #[test]
    fn wp_c7_a_rename_elsewhere_reports_names() {
        let (core, remote) = demo();
        remote.push(name_packet(EVT_DEVICE_NAME, 0, SRC_GPIO, b"Kitchen"));
        process_until(&core, |r| r.flags & NOTIFY_NAMES != 0);
        let log = core.monitor_read(0);
        assert!(
            log.entries.iter().any(|e| e.text.contains("\"Kitchen\"")),
            "{log:?}"
        );
        // Our own rename comes back HOST_SET: not news.
        remote.push(name_packet(EVT_INPUT_SOURCE_NAME, 1, SRC_HOST, b"TV"));
        assert_eq!(settle(&core).flags & NOTIFY_NAMES, 0);
    }

    #[test]
    fn wp_c7_an_undecodable_packet_re_reads() {
        let (core, remote) = demo();
        // A version the decoder refuses arrives as a lost packet.
        remote.push(vec![3, 2, 0, 0, 0, 0, 0, 0]);
        let r = process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        assert_eq!(r.flags & NOTIFY_STATE, NOTIFY_STATE);
    }

    #[test]
    fn wp_c7_an_offset_outside_the_image_re_reads() {
        let (core, remote) = demo();
        let mut p = vec![2, nfy::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
        p.extend(((g::BULK_SIZE - 2) as u16).to_le_bytes());
        p.extend(4u16.to_le_bytes());
        p.extend([SRC_GPIO, 0, 0, 0, 1, 2, 3, 4]);
        remote.push(p);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        // Nor is the header ever patched.
        let mut p = vec![2, nfy::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
        p.extend(0u16.to_le_bytes());
        p.extend(1u16.to_le_bytes());
        p.extend([SRC_GPIO, 0, 0, 0, 99]);
        remote.push(p);
        process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        assert_eq!(core.get_state().format_version, 32);
    }

    #[test]
    fn wp_c7_a_read_only_connection_re_reads_instead_of_patching() {
        let (core, remote) = demo();
        // As if the image had been another wire format's (§1.5).
        {
            let mut inner = core.lock();
            inner.conn().unwrap().wire = Some(dspi_proto::wire_compat::WireRead::Older);
            inner.state.bulk_valid = false;
        }
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-21.0f32).to_le_bytes(), SRC_UAC1));
        let r = process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        assert_eq!(r.flags & NOTIFY_STATE, NOTIFY_STATE);
        assert_eq!(core.get_state().master_volume_db, -21.0);
    }

    #[test]
    fn wp_c7_an_unplug_is_reported_and_an_intentional_close_is_not() {
        let (core, remote) = demo();
        remote.disconnect();
        let r = process_until(&core, |r| r.flags & NOTIFY_DISCONNECTED != 0);
        assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "no re-read of a gone device");
        assert!(core.notification_stream_ended());
        // Level-triggered until the connection is closed.
        assert_eq!(
            core.process_notifications().flags & NOTIFY_DISCONNECTED,
            NOTIFY_DISCONNECTED
        );
        core.disconnect();
        assert_eq!(core.process_notifications(), NotifyResult::default());
        assert!(!core.notification_stream_ended());

        let (core, _remote) = demo();
        core.disconnect();
        assert_eq!(core.process_notifications(), NotifyResult::default());
        assert!(!core.notification_stream_ended());
    }

    /// The fake behind a notification endpoint that loses the next packet
    /// while `drop_next` is set (the fake itself never skips a number).
    struct Lossy {
        dev: FakeDevice,
        drop_next: Arc<AtomicBool>,
    }

    struct LossySource {
        inner: Box<dyn NotificationSource>,
        drop_next: Arc<AtomicBool>,
    }

    impl NotificationSource for LossySource {
        fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
            let p = self.inner.read(timeout)?;
            if !p.is_empty() && p.len() > 1 && self.drop_next.swap(false, Ordering::SeqCst) {
                return Ok(Vec::new());
            }
            Ok(p)
        }
    }

    impl Transport for Lossy {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.dev.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.dev.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.dev.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            Some(Box::new(LossySource {
                inner: self.dev.notifications()?,
                drop_next: Arc::clone(&self.drop_next),
            }))
        }
        fn caps(&self) -> TransportCaps {
            self.dev.caps()
        }
    }

    #[test]
    fn wp_c7_a_sequence_gap_re_reads() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let drop_next = Arc::new(AtomicBool::new(false));
        let core = DspiCore::new();
        let t = Lossy {
            dev,
            drop_next: Arc::clone(&drop_next),
        };
        core.attach_probed(Box::new(t), "DEMO".into(), 0).unwrap();
        settle(&core);
        // One packet to start the sequence, one lost, one after the gap.
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-1.0f32).to_le_bytes(), SRC_UAC1));
        process_until(&core, |r| r.flags & NOTIFY_STATE != 0);
        drop_next.store(true, Ordering::SeqCst);
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-2.0f32).to_le_bytes(), SRC_UAC1));
        let start = Instant::now();
        while drop_next.load(Ordering::SeqCst) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(core.process_notifications().flags & NOTIFY_REFRESHED, 0);
        assert!(remote.poke(g::OFF_GLOBAL, &[1], SRC_UAC1)); // global.bypass
        let r = process_until(&core, |r| r.flags & NOTIFY_REFRESHED != 0);
        assert_eq!(r.flags & NOTIFY_CURVES, NOTIFY_CURVES);
        // The re-read brought the lost value in.
        assert_eq!(core.get_state().master_volume_db, -2.0);
    }

    /// The fake with control surfaces: it answers the caps (so the probe
    /// finds them); every other control-surface read stalls, which
    /// `cs_fetch` tolerates, leaving an empty configuration to show.
    struct WithCs {
        dev: FakeDevice,
    }

    impl WithCs {
        fn caps_reply(opcode: u8, value: u16, len: usize) -> Option<Vec<u8>> {
            if opcode != op::REQ_GET_CS_CAPS || value != 0xFFFF {
                return None;
            }
            let caps = dspi_session::cs_model::demo::caps();
            let mut b = dspi_proto::packets::CsCapsHeader {
                caps_version: caps.caps_version,
                max_bindings: caps.max_bindings,
                type_count: caps.type_count,
                noun_count: caps.noun_count,
                types: caps.types,
                max_ir_commands: caps.max_ir_commands,
                max_groups: caps.max_groups,
                max_macros: caps.max_macros,
                max_macro_steps: caps.max_macro_steps,
            }
            .encode();
            b.truncate(len);
            Some(b)
        }
    }

    impl Transport for WithCs {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            match Self::caps_reply(o, v, usize::from(l)) {
                Some(b) => Ok(b),
                None => self.dev.control_in(o, v, l),
            }
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            match Self::caps_reply(o, v, usize::from(l)) {
                Some(b) => Ok(b),
                None => self.dev.control_in_upto(o, v, l),
            }
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.dev.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.dev.descriptor()
        }
        fn max_transfer(&self) -> usize {
            self.dev.max_transfer()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            self.dev.notifications()
        }
        fn caps(&self) -> TransportCaps {
            self.dev.caps()
        }
    }

    #[test]
    fn wp_c7_ir_learn_and_foreign_aux_reach_the_cs_snapshot() {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(WithCs { dev }), "DEMO".into(), 0)
            .unwrap();
        settle(&core);
        core.cs_fetch().unwrap();
        let snap = |core: &DspiCore| -> serde_json::Value {
            serde_json::from_str(&core.cs_snapshot()).unwrap()
        };
        let before = snap(&core);
        assert_eq!(before["supported"], true, "{before}");
        assert!(before["learn"].is_null());
        assert_eq!(before["auxState"][3], 0);

        remote.push(ev(
            nfy::NOTIFY_EVT_CS_IR_LEARN,
            &[2, 1, 0, 0, 0xEF, 0x10, 0xDF, 0x20],
        ));
        remote.push(ev(nfy::NOTIFY_EVT_CS_AUX, &[3, 1, 0, 0x40, SRC_GPIO]));
        // Our own write and a slot past the table change nothing.
        remote.push(ev(nfy::NOTIFY_EVT_CS_AUX, &[4, 1, 0, 0x40, SRC_HOST]));
        remote.push(ev(nfy::NOTIFY_EVT_CS_AUX, &[200, 1, 0, 0x40, SRC_GPIO]));
        let want = NOTIFY_IR_LEARN | NOTIFY_CS_AUX;
        process_until(&core, |r| r.flags & want == want);
        settle(&core);

        let s = snap(&core);
        assert_eq!(s["learn"]["state"], 2, "{s}");
        assert_eq!(s["learn"]["protocol"], 1);
        assert_eq!(s["learn"]["code"], 0x20DF_10EFu32);
        assert_eq!(s["auxState"][3], 1);
        assert_eq!(s["auxLevel"][3], 0x4000);
        assert_eq!(s["auxState"][4], 0, "our own write is not noted");
        assert_eq!(s["auxLevel"][4], 0);

        // A later fetch keeps the learn result: only the device's aux is re-read.
        core.cs_fetch().unwrap();
        assert_eq!(snap(&core)["learn"]["code"], 0x20DF_10EFu32);
    }

    /// A listener that calls back into the core from the reader thread.
    struct Reentrant {
        core: Mutex<Option<Arc<DspiCore>>>,
        tx: Mutex<mpsc::Sender<NotifyResult>>,
    }

    impl NotifyListener for Reentrant {
        fn on_notifications(&self) {
            let core = self.core.lock().unwrap().clone();
            if let Some(core) = core {
                let r = core.process_notifications();
                let _ = core.get_state();
                let _ = core.monitor_read(0);
                let _ = self.tx.lock().unwrap().send(r);
            }
        }
    }

    #[test]
    fn wp_c7_a_listener_calling_back_into_the_core_does_not_deadlock() {
        let (core, remote) = demo();
        let (tx, rx) = mpsc::channel();
        let listener = Arc::new(Reentrant {
            core: Mutex::new(Some(Arc::clone(&core))),
            tx: Mutex::new(tx),
        });
        core.set_notify_listener(Some(listener.clone()));
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-5.0f32).to_le_bytes(), SRC_UAC1));
        loop {
            let r = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the listener ran");
            if r.flags & NOTIFY_STATE != 0 {
                break;
            }
        }
        assert_eq!(core.get_state().master_volume_db, -5.0);
        // A foreign bulk write makes the listener's call re-read too.
        remote.load_image(remote.image(), SRC_BULK);
        loop {
            let r = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the listener ran");
            if r.flags & NOTIFY_REFRESHED != 0 {
                break;
            }
        }
        core.set_notify_listener(None);
        // Break the cycle core -> listener -> core.
        listener.core.lock().unwrap().take();
    }

    #[test]
    fn wp_c7_graph_effect_classifies_offsets() {
        let mut ch = 0;
        assert_eq!(graph_effect(g::OFF_MASTER_VOLUME, &mut ch), 0);
        assert_eq!(graph_effect(g::OFF_GLOBAL, &mut ch), NOTIFY_CURVES);
        assert_eq!(graph_effect(g::OFF_OUTPUTS + 4, &mut ch), NOTIFY_CURVES);
        assert_eq!(graph_effect(g::OFF_PREAMP, &mut ch), NOTIFY_CURVES);
        assert_eq!(ch, 0);
        let last = g::OFF_CROSSOVERS + g::LEN_CROSSOVERS - 1;
        assert_eq!(graph_effect(last, &mut ch), NOTIFY_CURVES);
        assert_eq!(ch, 1 << 16);
        assert_eq!(graph_effect(g::OFF_EQ, &mut ch), NOTIFY_CURVES);
        assert_eq!(ch, 1 << 16 | 1);
    }

    #[test]
    fn wp_c7_patch_checks_its_range() {
        let mut img = vec![0u8; g::BULK_SIZE];
        assert_eq!(patch(&mut img, 0, &[1]), None);
        assert_eq!(patch(&mut img, g::BULK_SIZE, &[1]), None);
        assert_eq!(patch(&mut img, usize::MAX, &[1]), None);
        assert_eq!(patch(&mut img, g::OFF_GLOBAL, &[1]), Some(true));
        assert_eq!(patch(&mut img, g::OFF_GLOBAL, &[1]), Some(false));
    }
}

// -- WP CG25 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg25 {
    use std::time::{Duration, Instant};

    use dspi_transport::{FakeDevice, FakeRemote};

    use super::*;

    fn demo() -> (Arc<DspiCore>, FakeRemote) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        settle(&core);
        (core, remote)
    }

    fn settle(core: &DspiCore) -> u32 {
        let mut flags = 0;
        let mut quiet = 0;
        while quiet < 10 {
            std::thread::sleep(Duration::from_millis(10));
            let r = core.process_notifications();
            quiet = if r == NotifyResult::default() {
                quiet + 1
            } else {
                0
            };
            flags |= r.flags;
        }
        flags
    }

    /// Process until a flag in `want` shows up, then until quiet.
    fn flags_after(core: &DspiCore, want: u32) -> u32 {
        let start = Instant::now();
        let mut flags = 0;
        while flags & want == 0 {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            flags |= core.process_notifications().flags;
            std::thread::sleep(Duration::from_millis(5));
        }
        flags | settle(core)
    }

    fn ev(id: u16, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![2, id as u8, 0, 0];
        p.extend_from_slice(payload);
        p
    }

    fn adat(enabled: u8, active: u8, pin: u8) -> Vec<u8> {
        ev(nfy::NOTIFY_EVT_ADAT_STATE, &[enabled, active, pin])
    }

    #[test]
    fn wp_cg25_streams_is_a_new_bit() {
        let old = [
            NOTIFY_STATE,
            NOTIFY_CURVES,
            NOTIFY_REFRESHED,
            NOTIFY_INPUT_FORMAT,
            NOTIFY_PRESET,
            NOTIFY_SIGGEN,
            NOTIFY_CS_AUX,
            NOTIFY_IR_LEARN,
            NOTIFY_NAMES,
            NOTIFY_DISCONNECTED,
            NOTIFY_STALE,
        ];
        assert_eq!(NOTIFY_STREAMS, 1 << 11);
        assert!(old.iter().all(|f| f & NOTIFY_STREAMS == 0));
    }

    #[test]
    fn wp_cg25_adat_state_sets_the_flag_and_the_mirror_follows() {
        let (core, remote) = demo();
        let pin = core.get_state().adat_out_pin.wrapping_add(1).max(1);
        remote.push(adat(1, 1, pin));
        let f = flags_after(&core, NOTIFY_STREAMS);
        assert_eq!(f & NOTIFY_STATE, NOTIFY_STATE, "the mirror changed");
        let s = core.get_state();
        assert!(s.adat_out_enabled);
        assert_eq!(s.adat_out_pin, pin);

        // Pin 0 leaves the pin alone (as the Mac does).
        remote.push(adat(0, 0, 0));
        flags_after(&core, NOTIFY_STREAMS);
        let s = core.get_state();
        assert!(!s.adat_out_enabled);
        assert_eq!(s.adat_out_pin, pin);

        // Nothing changed: the flag, but no state change.
        remote.push(adat(0, 1, pin));
        let f = flags_after(&core, NOTIFY_STREAMS);
        assert_eq!(f & NOTIFY_STATE, 0);
    }

    #[test]
    fn wp_cg25_i2s_slave_and_adat_input_set_the_flag() {
        let (core, remote) = demo();
        remote.push(ev(nfy::NOTIFY_EVT_I2S_SLAVE_STATE, &[3, 0x80, 0xBB, 0, 0]));
        let f = flags_after(&core, NOTIFY_STREAMS);
        assert_eq!(f & NOTIFY_STATE, 0, "not a mirrored field");
        remote.push(ev(
            nfy::NOTIFY_EVT_ADAT_INPUT_STATE,
            &[3, 0x80, 0xBB, 0, 0, 1],
        ));
        let f = flags_after(&core, NOTIFY_STREAMS);
        assert_eq!(f & NOTIFY_STATE, 0);
    }

    #[test]
    fn wp_cg25_adat_state_survives_a_patch_in_the_same_drain() {
        let (core, remote) = demo();
        let pin = core.get_state().adat_out_pin.wrapping_add(1).max(1);
        let src = nfy::PARAM_SRC_UAC1 as u8;
        // Parameter, ADAT state, parameter: one image is started before the
        // event and one after it, whatever the reader delivers per drain.
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-12.0f32).to_le_bytes(), src));
        remote.push(adat(1, 1, pin));
        assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-6.0f32).to_le_bytes(), src));
        std::thread::sleep(Duration::from_millis(50));
        flags_after(&core, NOTIFY_STREAMS);
        let s = core.get_state();
        assert!(s.adat_out_enabled);
        assert_eq!(s.adat_out_pin, pin);
        assert_eq!(s.master_volume_db, -6.0);
    }
}

// -- WP CG31 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_cg31;

// -- WP CG33 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_cg33;

// -- WP P1EFIX2 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_p1efix2;

// -- WP ECHO1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_echo1;
