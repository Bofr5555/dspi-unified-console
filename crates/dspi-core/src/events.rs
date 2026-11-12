//! Meter ballistics and the event history in the core (WP P0M, PRO1 M2 and
//! M1's client half).
//!
//! **Meters.** [`DspiCore::fetch_status`] runs every read through one
//! [`Ballistics`] (`dspi_session::meters`): `SystemStatus` carries the held
//! peaks, each channel's maximum since [`DspiCore::reset_meter_max`], and the
//! clip hold. When the hold ends the core clears the device's sticky flags
//! itself, as the macOS console does, so a client needs no clip timer of its
//! own. A stale read (a stalled Link feed) leaves the ballistics as they were.
//!
//! **History.** What the core sees is written to a per-device log
//! (`dspi_session::events`): notifications with their source and Link session,
//! connects and disconnects, clip episodes, temperature crossings, counters
//! stepping, restarts, firmware changes, preset recalls and imports with what
//! they changed. Nothing is polled for it: each record comes from a call a
//! client makes anyway (`fetch_status`, `fetch_stat`, `fetch_ring_health`,
//! `fetch_core_vreg`, the notification drain). In memory until a client
//! names its data directory with [`DspiCore::event_log_set_dir`].
//!
//! Locks: [`History`] has two, `meters` and `log`, never held together (the
//! meters' guard goes before a clip is logged). Either may be taken with or
//! without `Inner`, never `Inner` after them. A sink runs under `log`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use dspi_session::events::{
    EventKind, EventLog, EventRecord, EventSink, ExtEvent, ReadBatch, Watch, decode_ext_event,
    restarted,
};
use dspi_session::meters::{Ballistics, BallisticsConfig, MeterUpdate};
use dspi_session::notify::{Event, Notification};
use dspi_transport::clock::{self, Instant};

use crate::consts::*;
use crate::records::{DspState, EventBatch, EventEntry, MeterBallistics, RingHealth, SystemStatus};
use crate::{DspiCore, DspiError, Result};

/// The most records one [`DspiCore::event_log_read`] returns.
pub const EVENT_READ_MAX: u32 = 5000;

/// The meter state and the event history, shared by [`DspiCore`] and `Inner`.
#[derive(Default)]
pub(crate) struct History {
    meters: Mutex<MeterState>,
    log: Mutex<LogState>,
}

#[derive(Default)]
struct MeterState {
    ballistics: Ballistics,
    /// The clip hold ended and the device's flags still want clearing.
    clear_wanted: bool,
}

#[derive(Default)]
struct LogState {
    log: EventLog,
    watch: Watch,
    /// Input channels of the open device, to name a clipped channel.
    inputs: u8,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Now, ms since the epoch, through the clock seam.
pub(crate) fn now_ms() -> i64 {
    clock::system_time()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// What the history wants to know of a connection that just opened.
pub(crate) struct Opened {
    pub(crate) serial: String,
    pub(crate) transport: String,
    pub(crate) fw: Option<String>,
    pub(crate) inputs: u8,
    /// Uptime and hang count, from the core-voltage extension's probe.
    pub(crate) uptime: Option<(u32, u8)>,
}

impl Opened {
    /// From the installed mirror and connection facts.
    pub(crate) fn new(
        serial: &str,
        kind: dspi_transport::TransportKind,
        state: &DspState,
        vreg: Option<(u32, u8)>,
    ) -> Self {
        let fw = (state.fw_major != 0 || state.fw_minor != 0 || state.fw_patch != 0).then(|| {
            let mut v = format!("{}.{}.{}", state.fw_major, state.fw_minor, state.fw_patch);
            if state.fw_beta != 0 {
                v.push_str(&format!(" beta {}", state.fw_beta));
            }
            v
        });
        Self {
            serial: serial.to_string(),
            transport: format!("{kind:?}").to_lowercase(),
            fw,
            inputs: state.num_input_channels,
            uptime: vreg,
        }
    }
}

impl History {
    // ── Meters ────────────────────────────────────────────────────────

    /// Run `status` through the ballistics and fill its hold fields. Returns
    /// whether the device's clip flags should be cleared now.
    pub(crate) fn meters(&self, status: &mut SystemStatus) -> bool {
        let now = Instant::now();
        let (update, clear) = {
            let mut m = lock(&self.meters);
            let update = if status.stale {
                MeterUpdate::default()
            } else {
                m.ballistics.update(now, &status.peaks, status.clip_flags)
            };
            m.clear_wanted |= update.clear_due;
            status.peak_hold = m.ballistics.peak_hold();
            status.peak_max = m.ballistics.peak_max();
            status.clip_hold = m.ballistics.clip_hold();
            (update, m.clear_wanted)
        };
        if update.new_clips != 0 {
            self.clips(update.new_clips, "meter");
        }
        clear
    }

    /// The device's flags were cleared (or the connection went).
    pub(crate) fn clips_cleared(&self) {
        lock(&self.meters).clear_wanted = false;
    }

    /// A user's clear: the hold goes now.
    pub(crate) fn clear_clip_hold(&self) {
        let mut m = lock(&self.meters);
        m.ballistics.clear_clips();
        m.clear_wanted = false;
    }

    fn reset_meters(&self) {
        let mut m = lock(&self.meters);
        m.ballistics.reset();
        m.clear_wanted = false;
    }

    // ── Records ───────────────────────────────────────────────────────

    fn record(&self, record: EventRecord) {
        let mut l = lock(&self.log);
        l.log.tick(record.t);
        l.log.record(record);
    }

    fn clips(&self, mask: u32, source: &str) {
        let t = now_ms();
        let mut l = lock(&self.log);
        let inputs = l.inputs;
        for ch in 0..32u8 {
            if mask & 1u32.checked_shl(u32::from(ch)).unwrap_or(0) == 0 {
                continue;
            }
            let mut r = EventRecord::new(
                EventKind::Clip,
                t,
                format!("Clip on {}", channel_name(ch, inputs)),
            );
            r.channel = Some(ch);
            r.source = Some(source.to_string());
            l.log.record(r);
        }
    }

    /// A connection opened: the device's log becomes current, and its last
    /// connection is compared for a firmware change and a restart.
    pub(crate) fn connected(&self, o: Opened) {
        self.reset_meters();
        let t = now_ms();
        let mut l = lock(&self.log);
        let last = l.log.open_device(&o.serial);
        l.watch = Watch::new();
        l.inputs = o.inputs;
        let mut r = EventRecord::new(
            EventKind::Connected,
            t,
            match &o.fw {
                Some(fw) => format!("Connected over {} (firmware {fw})", o.transport),
                None => format!("Connected over {}", o.transport),
            },
        );
        r.source = Some(o.transport.clone());
        r.fw.clone_from(&o.fw);
        r.uptime_s = o.uptime.map(|(u, _)| u);
        l.log.record(r);
        if let (Some(was), Some(now)) = (&last.fw, &o.fw)
            && was != now
        {
            let mut r = EventRecord::new(
                EventKind::Firmware,
                t,
                format!("Firmware changed from {was} to {now}"),
            );
            r.fw = Some(now.clone());
            r.lines = vec![format!("was {was}")];
            l.log.record(r);
        }
        if let Some((uptime, hangs)) = o.uptime {
            if last.uptime.is_some_and(|prev| restarted(prev, uptime, t)) {
                let mut r = EventRecord::new(
                    EventKind::Reset,
                    t,
                    format!("The device restarted while away; up {uptime} s"),
                );
                r.source = Some("unknown".into());
                r.uptime_s = Some(uptime);
                l.log.record(r);
            }
            // The connection's baseline.
            let _ = l.watch.uptime(uptime, Some(hangs), t);
        }
    }

    /// The connection to `serial` went (`reason`: closed, replaced,
    /// unplugged, link lost). Nothing when the history has moved on to
    /// another device meanwhile (a connect on another thread won the race).
    pub(crate) fn disconnected(&self, serial: &str, reason: &str) {
        if lock(&self.log).log.device() != Some(serial) {
            return;
        }
        // `meters` before `log`, never inside it.
        self.reset_meters();
        let mut l = lock(&self.log);
        if l.log.device() != Some(serial) {
            return;
        }
        let mut r = EventRecord::new(
            EventKind::Disconnected,
            now_ms(),
            format!("Disconnected ({reason})"),
        );
        r.source = Some(reason.to_string());
        l.log.record(r);
        l.log.close_device();
    }

    /// A drained notification, with its monitor line. `own`: this client's
    /// write coming back. `features`: the device's `EXT_NOTIFY_FEATURES`.
    pub(crate) fn notification(&self, note: &Notification, own: bool, features: Option<u32>) {
        let t = crate::raw::epoch_ms(note.received);
        let (source, key) = match &note.event {
            Event::Idle => return,
            Event::ParamChanged { offset, source, .. } => (
                Some(*source),
                Some(format!("p{offset}:{source:?}:{:?}:{own}", note.origin)),
            ),
            Event::CsAux { slot, source, .. } => (
                Some(*source),
                Some(format!("aux{slot}:{source:?}:{:?}", note.origin)),
            ),
            Event::BulkInvalidated { source } => (Some(*source), None),
            Event::Unknown { id, bytes } => {
                // -- WP FWX hook: the extension's clip and reset events.
                match decode_ext_event(features, *id, bytes) {
                    Some(ExtEvent::Clip { channel }) => {
                        let bit = 1u32.checked_shl(u32::from(channel)).unwrap_or(0);
                        let new = lock(&self.meters)
                            .ballistics
                            .note_clips(Instant::now(), bit);
                        if new.new_clips != 0 {
                            self.clips(new.new_clips, "notification");
                        }
                        return;
                    }
                    Some(ExtEvent::Reset { reason }) => {
                        let mut r = EventRecord::new(
                            EventKind::Reset,
                            t,
                            format!("The device restarted ({})", reason.as_str()),
                        );
                        r.source = Some(reason.as_str().to_string());
                        self.record(r);
                        return;
                    }
                    None => (None, None),
                }
            }
            _ => (None, None),
        };
        let mut r = EventRecord::new(EventKind::Notification, t, crate::monitor::describe(note));
        r.source = source.map(crate::monitor::source_label);
        r.session = note.origin.filter(|&o| o != 0).map(u32::from);
        r.own = own;
        r.coalesce = key;
        self.record(r);
    }

    /// A `REQ_GET_STATUS` counter or reading the client polled.
    pub(crate) fn stat(&self, which: u16, value: u32) {
        let t = now_ms();
        let mut l = lock(&self.log);
        let r = if which == STAT_TEMPERATURE {
            // centi-°C, signed in the firmware's word.
            l.watch.temperature(value as i32, t)
        } else {
            let Some((name, label)) = counter_name(which) else {
                return;
            };
            l.watch.counter(name, label, u64::from(value), t)
        };
        if let Some(r) = r {
            l.log.tick(t);
            l.log.record(r);
        }
    }

    /// The output rings' counters.
    pub(crate) fn ring_health(&self, h: &RingHealth) {
        let t = now_ms();
        let mut l = lock(&self.log);
        for (name, label, v) in [
            (
                "ring.underrun_episodes",
                "Output ring underruns",
                h.underrun_episodes,
            ),
            (
                "ring.refused_blocks",
                "Output ring refused blocks",
                h.refused_blocks,
            ),
            (
                "ring.over_budget_steps",
                "Processing over budget",
                h.over_budget_steps,
            ),
        ] {
            if let Some(r) = l.watch.counter(name, label, u64::from(v), t) {
                l.log.record(r);
            }
        }
    }

    /// The core-voltage extension's uptime and hang count.
    pub(crate) fn uptime(&self, uptime_s: u32, hang_count: u8) {
        let t = now_ms();
        let mut l = lock(&self.log);
        if let Some(r) = l.watch.uptime(uptime_s, Some(hang_count), t) {
            l.log.record(r);
        }
    }

    /// A preset recall (`slot`) or an import (`slot` None, `what` the
    /// document), with what it changed.
    pub(crate) fn applied(
        &self,
        kind: EventKind,
        slot: Option<u8>,
        what: &str,
        lines: Vec<String>,
    ) {
        let n = lines.len();
        let changed = match n {
            0 => "no changes".to_string(),
            1 => "1 change".to_string(),
            n => format!("{n} changes"),
        };
        let mut r = EventRecord::new(kind, now_ms(), format!("{what}: {changed}"));
        r.channel = slot;
        r.lines = lines;
        r.count = u32::try_from(n).unwrap_or(u32::MAX).max(1);
        self.record(r);
    }
}

/// `IN n` or `OUT n` for wire channel `ch` on a device with `inputs` inputs.
fn channel_name(ch: u8, inputs: u8) -> String {
    match ch.checked_sub(inputs) {
        Some(o) if inputs > 0 => format!("output {}", u16::from(o).saturating_add(1)),
        _ => format!("input {}", u16::from(ch).saturating_add(1)),
    }
}

/// The counters the history follows: a stable key and the text's name.
fn counter_name(which: u16) -> Option<(&'static str, &'static str)> {
    Some(match which {
        STAT_PDM_RING_OVERRUNS => ("pdm.ring_overruns", "PDM ring overruns"),
        STAT_PDM_RING_UNDERRUNS => ("pdm.ring_underruns", "PDM ring underruns"),
        STAT_PDM_DMA_OVERRUNS => ("pdm.dma_overruns", "PDM DMA overruns"),
        STAT_PDM_DMA_UNDERRUNS => ("pdm.dma_underruns", "PDM DMA underruns"),
        STAT_SPDIF_OVERRUNS => ("spdif.overruns", "S/PDIF overruns"),
        STAT_SPDIF_UNDERRUNS => ("spdif.underruns", "S/PDIF underruns"),
        STAT_STARVATION_TOTAL => ("starvation", "Audio starvation"),
        STAT_USB_RING_OVERRUNS => ("usb.ring_overruns", "USB ring overruns"),
        _ => return None,
    })
}

fn entry(r: EventRecord) -> EventEntry {
    EventEntry {
        id: r.id,
        time_ms: r.t,
        kind: r.kind.as_str().to_string(),
        serial: r.serial,
        text: r.text,
        source: r.source.unwrap_or_default(),
        session: r.session,
        own: r.own,
        channel: r.channel,
        value: r.value,
        fw: r.fw,
        uptime_s: r.uptime_s,
        lines: r.lines,
        count: r.count,
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    // -- Meters --

    /// The meter ballistics' timing (`SystemStatus.peak_hold`, `clip_hold`).
    /// Defaults: hold 1000 ms, then a fall of 0.5 full scale per second; clip
    /// hold 10 000 ms after the last new clip. Never fails.
    pub fn meter_ballistics(&self) -> MeterBallistics {
        let c = lock(&self.history.meters).ballistics.config();
        MeterBallistics {
            hold_ms: c.hold_ms,
            decay_per_s: c.decay_per_s,
            clip_hold_ms: c.clip_hold_ms,
        }
    }

    /// Change the meter ballistics' timing for every later `fetch_status`.
    /// What is held stays. A `decay_per_s` below 0 or not finite reads as 0
    /// (a held peak then stays until a higher one); a `clip_hold_ms` of 0
    /// holds clips until `clear_clips`. Never fails.
    pub fn set_meter_ballistics(&self, config: MeterBallistics) {
        lock(&self.history.meters)
            .ballistics
            .set_config(BallisticsConfig {
                hold_ms: config.hold_ms,
                decay_per_s: config.decay_per_s,
                clip_hold_ms: config.clip_hold_ms,
            });
    }

    /// Start every channel's `SystemStatus.peak_max` again from the next
    /// read. A new connection does this too. Never fails.
    pub fn reset_meter_max(&self) {
        lock(&self.history.meters).ballistics.reset_max();
    }

    // -- Event history --

    /// Keep the event history in files under `dir`, the client's own data
    /// directory, created if missing: one JSON-lines file per device serial,
    /// rotated, at most about 1 MiB per device. An empty `dir` keeps it in
    /// memory only (the default; the last 1000 records). Call it at start-up,
    /// before connecting; the open device's history carries on in the new
    /// place. Blocking file I/O: not on the UI thread.
    ///
    /// Errors: [`DspiError::Io`] when the directory cannot be created;
    /// [`DspiError::Unsupported`] on the web, which keeps it in memory.
    pub fn event_log_set_dir(&self, dir: String) -> Result<()> {
        let dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
        lock(&self.history.log).log.set_dir(dir).map_err(|e| {
            if e.kind() == std::io::ErrorKind::Unsupported {
                DspiError::Unsupported
            } else {
                DspiError::Io {
                    msg: format!("the event history's folder: {e}"),
                }
            }
        })
    }

    /// `serial`'s event history with ids above `after_id`, oldest first, at
    /// most `limit` (capped at [`EVENT_READ_MAX`]): with `after_id` 0 the
    /// newest `limit`, otherwise the next `limit` after it, so poll with the
    /// batch's `last_id`. An empty `serial` is the open device. Lines in the
    /// file that are not records are skipped and counted. Reads files: not on
    /// the UI thread. Never fails; an unknown serial reads empty.
    pub fn event_log_read(&self, serial: String, after_id: u64, limit: u32) -> EventBatch {
        let mut l = lock(&self.history.log);
        let serial = self.history_serial(&l, serial);
        let limit = usize::try_from(limit.min(EVENT_READ_MAX)).unwrap_or(0);
        let page = l.log.read(&serial, after_id, limit);
        EventBatch {
            last_id: page.last_id,
            skipped: page.skipped,
            entries: page.records.into_iter().map(entry).collect(),
        }
    }

    /// `serial`'s whole event history as JSON lines, oldest first, in the
    /// file format (`docs` in the WP P0M report): for a "Save history…" or a
    /// diagnostics bundle. An empty `serial` is the open device. Never fails;
    /// an unknown serial exports "".
    pub fn event_log_export(&self, serial: String) -> String {
        let mut l = lock(&self.history.log);
        let serial = self.history_serial(&l, serial);
        l.log.export(&serial)
    }

    /// The serials with an event history, sorted. Never fails.
    pub fn event_log_devices(&self) -> Vec<String> {
        lock(&self.history.log).log.devices()
    }
}

impl DspiCore {
    /// [`DspiCore::event_log_read`] as the history's own records, which
    /// serialise in the file format: what the C ABI and wasm hand out as JSON
    /// lines.
    pub fn event_log_page(&self, serial: String, after_id: u64, limit: u32) -> ReadBatch {
        let mut l = lock(&self.history.log);
        let serial = self.history_serial(&l, serial);
        let limit = usize::try_from(limit.min(EVENT_READ_MAX)).unwrap_or(0);
        l.log.read(&serial, after_id, limit)
    }

    /// Send every history record to `sink` as well (the hub's shared history,
    /// M1's hub half), or stop with `None`. The sink runs with the history's
    /// lock held: it must not call into this core or block.
    pub fn set_event_sink(&self, sink: Option<Arc<dyn EventSink>>) {
        lock(&self.history.log).log.set_sink(sink);
    }

    /// An empty `serial` is the open device's, else the last one's.
    fn history_serial(&self, l: &LogState, serial: String) -> String {
        if serial.is_empty() {
            l.log.device().map(str::to_string).unwrap_or_default()
        } else {
            serial
        }
    }

    /// Clear the device's sticky clip flags after the clip hold ended, without
    /// waiting: when another call holds the lock, the next read tries again.
    pub(crate) fn clear_clips_when_free(&self) {
        let Some(mut inner) = self.try_lock() else {
            return;
        };
        let Ok(conn) = inner.conn() else {
            self.history.clips_cleared();
            return;
        };
        // Refused (a viewer on a hub) or failed: the hold stays dark until the
        // device reports the bits clear; do not retry every read.
        let _ = conn.get_upto(dspi_proto::generated::opcodes::REQ_CLEAR_CLIPS, 0, 4);
        drop(inner);
        self.history.clips_cleared();
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p0m;
