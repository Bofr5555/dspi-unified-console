//! The hub's shared event history and its watchdog events (WP P2EV, PLAN-P2 M1's hub
//! half and S6; `docs/integrator/hub-api.md` section 6).
//!
//! One log per device in `<data>/events/`, written by
//! [`dspi_session::events::EventStore`] (WP P2EVW) in the clients' own file format and
//! lock rule (`docs/integrator/event-log.md`), so `EventLog::export_in` and the
//! diagnostics bundle read the folder unchanged. The history writes what the hub sees on
//! its bus ([`crate::bus`]), on a thread of its own (appends are file I/O, never on a
//! device worker or the `hub-system` timer):
//!
//! | Bus message | Record |
//! |---|---|
//! | `DeviceOnline`, `DeviceOffline` | `connected`, `disconnected` |
//! | `Notification` | `notification` (`session`, `by` the writer's label), coalesced as the format does (a slider sweep is one record with a `count`); an extension clip event (0xE0) `clip`, the audio link's 0xE1-0xE3 `audio_link` |
//! | `Facts` | `reset` with its reason, when the device restarted since the log last saw it (by its uptime); `firmware` when the version differs from the log's (one record with the inventory's `Event` of the same change, P2IN). The health watch then starts from the facts' uptime, so one restart is one `reset` |
//! | `Health` | `temperature` when it crosses a 5 °C band, `reset` when a new uptime (it refreshes every 60 s; a repeat is no reading) went back or the hang count rose (the counters), `clip` from the sticky latch on firmware without clip events (never cleared by the hub) |
//! | `Unresponsive` | `unresponsive` (`value` the seconds silent) |
//! | `Alarm` (a device's) | `alarm` (`source` its code, `by` who acknowledged it) |
//! | `ScheduleRan` | `schedule`, one per device in its results (`by` the rule) |
//! | `Event` with `record.id == 0` | that record, as given: **how another service records something** (identify: `notification` with source `identify`, P2ID; a firmware change: `firmware`, P2IN; a recall, P2PA). A `firmware` record that names the firmware the log already has is left out, so two services seeing one change write one record |
//!
//! Every record written goes back on the bus as `Event` with its id (above 0), and to every
//! session watching (`event.added`). Messages the history's bus queue dropped, and records
//! the store could not write, become one `events_dropped {count}` record in the log of each
//! device online (of each device seen, when none is), never silence: one record per loss
//! episode, written once nothing was lost for a second (or each minute while it lasts). The
//! history takes no written records (`Event` with an id) from the bus, so its own echoes
//! never fill its queue or count as lost.
//!
//! **Messages** (`event.*`, cap `events`, answers `"v": 1`):
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `event.list {serial or handle, after_id?, limit?}` | every role | `serial`, `records` (oldest first; with `after_id` 0 the newest `limit`), `more`, `newest_id` |
//! | `event.watch {on, serial?}` | every role | `on`, `serial`; then `event.added {v, hub, serial, record}` until `on: false` or the session ends |
//! | `event.export {serial or handle, after_id?}` | Admin | `serial`, `text` (whole records, the file format, at most about 48 KiB), `next_after_id` (`null` at the end), `newest_id` |
//!
//! **Ids may start again at 1**: after `inventory.forget` removes a device's files, or when
//! the total cap removed a device's every file and the hub restarted. `newest_id` is the
//! highest id the log holds now; a client paging with an `after_id` above it knows the
//! numbering started again and reads from 0.
//!
//! Off unless [`crate::ServiceConfig::history`] asks for it, and only on a hub with a data
//! folder.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dspi_proto::ext::ResetInfo;
use dspi_proto::generated::SECTIONS;
use dspi_session::events::{
    self, COALESCE_MS, EventKind, EventRecord, EventStore, ExtEvent, HUB_LIMITS, HUB_TOTAL_BYTES,
    Limits, Watch, alink_record, decode_ext_event,
};
use dspi_session::notify::{self, Event, Source};
use serde_json::{Value, json};

use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Outbound, Outbox, Role, Session, SessionId};
use crate::system::alarms::AlarmState;

/// The family's cap in `hello.caps`.
pub const CAP: &str = "events";
/// The folder under the hub's data folder.
pub const DIR_NAME: &str = "events";
/// Records in a page when the request does not say (hub-api.md 1.8).
pub const PAGE_DEFAULT: usize = 100;
/// The most records in one page.
pub const PAGE_MAX: usize = 500;
/// About how much text one `event.export` answer carries: whole records up to this.
pub const EXPORT_PAGE_BYTES: usize = 48 * 1024;
/// Sessions watching at once.
pub const MAX_WATCHERS: usize = 256;
/// Devices the history follows in memory (the inventory's cap); past it the one seen
/// longest ago that is offline is forgotten (its files stay).
pub const MAX_DEVICES: usize = 1024;
/// The temperature bands, °C.
pub const TEMP_BAND_C: f32 = 5.0;
/// How far past a band's edge a reading must go to leave it, °C, so a reading on an edge
/// does not flap.
pub const TEMP_HYSTERESIS_C: f32 = 1.0;
/// Coalescing records held back per device at once.
pub const MAX_HELD: usize = 16;
/// Messages the history's bus queue holds unless configured otherwise.
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// How long the thread waits on the bus before it looks at the coalescing records.
const POLL: Duration = Duration::from_millis(200);
/// A loss episode ends when nothing more was lost for this long, ms; its count is then
/// written as one `events_dropped`.
const EPISODE_QUIET_MS: i64 = 1000;
/// While a loss episode lasts, its count is written this often, ms.
const EPISODE_MAX_MS: i64 = 60_000;
/// How often a lost count that could not be written is tried again, ms.
const LOST_RETRY_MS: i64 = 1000;
const VERSION: u64 = 1;

/// The history's settings (OD-P2k's bounds by default).
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    /// Per device: 512 KiB per file and three older files.
    pub limits: Limits,
    /// Over every device's log: 128 MiB.
    pub total_bytes: u64,
    /// The history's bus queue (clamped by the bus to 1..=4096).
    pub bus_capacity: usize,
    /// For tests: while it reads `true` the history does not read the bus, so a test can
    /// fill its queue. `None` in a real hub.
    #[doc(hidden)]
    pub test_hold: Option<Arc<AtomicBool>>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            limits: HUB_LIMITS,
            total_bytes: HUB_TOTAL_BYTES,
            bus_capacity: DEFAULT_BUS_CAPACITY,
            test_hold: None,
        }
    }
}

/// The service for [`crate::service`]'s list: `None` when the history is off, the hub has
/// no data folder, or its folder cannot be made.
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    let history = config.history.as_ref()?;
    let Some(data) = setup.data_dir else {
        log::info!("hub: the event history needs a data folder; it is off");
        return None;
    };
    match HistoryService::start(history, data.join(DIR_NAME), setup.clock, setup.bus) {
        Ok(s) => Some(s),
        Err(e) => {
            log::error!("hub: the event history is off: {e}");
            None
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── The pure state ──────────────────────────────────────────────────────────

/// The time a message is handled at: the wall clock for records, the monotonic one for
/// the uptime watch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Now {
    pub(crate) ms: i64,
    pub(crate) mono: Instant,
}

/// What the history keeps of one device.
#[derive(Debug, Default)]
struct Device {
    online: bool,
    /// Its firmware has the extension (its `Facts` carry `EXT_IDENTIFY` or
    /// `EXT_RESET_INFO`), so its 0xE0-0xE3 notifications are the extension's events.
    ext_events: bool,
    /// The log was read for what follows.
    loaded: bool,
    /// This history wrote to its log since it was last forgotten.
    wrote: bool,
    /// The firmware the log last named.
    fw: Option<String>,
    /// The log's last uptime: when, and the uptime then.
    uptime: Option<(i64, u32)>,
    /// A record held back for coalescing.
    pending: Vec<EventRecord>,
    // Per connection:
    watch: Watch,
    temp_band: Option<i32>,
    clip_latch: Option<u32>,
    /// The uptime of the last `Health`: a repeat is the same reset-info read, not news.
    health_uptime: Option<u32>,
    /// When the history last heard of it, ms.
    seen_ms: i64,
}

/// The history: the store and what it remembers per device. No threads, no sockets; the
/// caller passes the time.
pub(crate) struct History {
    store: EventStore,
    devices: BTreeMap<String, Device>,
    /// Records lost (bus drops, failed writes) not yet written as `events_dropped`.
    lost: u64,
    /// When the loss episode began (its first loss not yet written), ms.
    lost_since_ms: Option<i64>,
    /// The episode's latest loss, ms.
    lost_last_ms: i64,
    lost_tried_ms: Option<i64>,
}

impl History {
    pub(crate) fn open(
        dir: PathBuf,
        config: &HistoryConfig,
    ) -> Result<Self, events::EventStoreError> {
        Ok(Self {
            store: EventStore::open(dir, config.limits, config.total_bytes)?,
            devices: BTreeMap::new(),
            lost: 0,
            lost_since_ms: None,
            lost_last_ms: 0,
            lost_tried_ms: None,
        })
    }

    /// The device's entry, made (and its log read) on first sight.
    fn device(&mut self, serial: &str, now: Now) -> &mut Device {
        if !self.devices.contains_key(serial) {
            if self.devices.len() >= MAX_DEVICES {
                let oldest = self
                    .devices
                    .iter()
                    .filter(|(_, d)| !d.online && d.pending.is_empty())
                    .min_by_key(|(_, d)| d.seen_ms)
                    .map(|(s, _)| s.clone());
                if let Some(s) = oldest {
                    self.devices.remove(&s);
                }
            }
            self.devices.insert(serial.to_owned(), Device::default());
        }
        let store = &self.store;
        let d = self.devices.entry(serial.to_owned()).or_default();
        if !d.loaded {
            d.loaded = true;
            let all = store.read(serial, 0, usize::MAX).records;
            d.fw = all.iter().rev().find_map(|r| r.fw.clone());
            d.uptime = all.iter().rev().find_map(|r| r.uptime_s.map(|u| (r.t, u)));
        }
        d.seen_ms = now.ms;
        d
    }

    /// Handle one bus message; the records written, in order.
    pub(crate) fn on_message(&mut self, msg: &BusMessage, now: Now) -> Vec<EventRecord> {
        let mut out = Vec::new();
        match msg {
            BusMessage::DeviceOnline { serial, handle } => {
                let d = self.device(serial, now);
                d.online = true;
                d.watch = Watch::new();
                d.temp_band = None;
                d.clip_latch = None;
                d.health_uptime = None;
                let mut r = EventRecord::new(EventKind::Connected, now.ms, "Attached to the hub");
                r.source = Some(format!("hub, handle {handle}"));
                out.extend(self.push(serial, r, now));
            }
            BusMessage::DeviceOffline { serial, .. } => {
                self.device(serial, now).online = false;
                let mut r =
                    EventRecord::new(EventKind::Disconnected, now.ms, "Detached from the hub");
                r.source = Some("detached".into());
                out.extend(self.push(serial, r, now));
                out.extend(self.flush(serial));
            }
            BusMessage::DeviceRemoved { .. } => {}
            BusMessage::Notification {
                serial,
                session,
                by,
                packet,
            } => {
                let ext = self.device(serial, now).ext_events;
                for r in notification_records(packet, *session, by.as_deref(), ext, now.ms) {
                    out.extend(self.push(serial, r, now));
                }
            }
            BusMessage::Facts {
                serial,
                identify,
                reset_info,
                fw,
            } => {
                let records = self.facts(serial, identify.is_some(), reset_info.as_ref(), fw, now);
                for r in records {
                    out.extend(self.push(serial, r, now));
                }
            }
            BusMessage::Health {
                serial,
                clip_mask,
                temp_c,
                uptime_s,
                hang_count,
                ..
            } => {
                let records = self.health(serial, *clip_mask, *temp_c, *uptime_s, *hang_count, now);
                for r in records {
                    out.extend(self.push(serial, r, now));
                }
            }
            BusMessage::Unresponsive { serial, since } => {
                let now_s = u64::try_from(now.ms / 1000).unwrap_or(0);
                let silent = now_s.saturating_sub(*since);
                let mut r = EventRecord::new(
                    EventKind::Unresponsive,
                    now.ms,
                    format!("Not answering (for {silent} s)"),
                );
                r.value = Some(silent as f64);
                out.extend(self.push(serial, r, now));
            }
            BusMessage::Alarm {
                code,
                serial: Some(serial),
                state,
                message,
                acknowledged_by,
                ..
            } => {
                let what = match state {
                    AlarmState::Raised => "raised",
                    AlarmState::Acknowledged => "acknowledged",
                    AlarmState::Cleared => "cleared",
                };
                let mut r =
                    EventRecord::new(EventKind::Alarm, now.ms, format!("Alarm {what}: {message}"));
                r.source = Some(code.clone());
                if matches!(state, AlarmState::Acknowledged) {
                    r.by.clone_from(acknowledged_by);
                }
                out.extend(self.push(serial, r, now));
            }
            BusMessage::Alarm { serial: None, .. } => {}
            BusMessage::ScheduleRan {
                rule,
                name,
                by,
                results,
                ..
            } => {
                let mut per: BTreeMap<&str, Vec<String>> = BTreeMap::new();
                let mut failed: BTreeMap<&str, bool> = BTreeMap::new();
                for t in results {
                    let action = t.action.as_deref().unwrap_or("action");
                    let line = match (&t.code, &t.msg) {
                        _ if t.ok => format!("{action}: ok"),
                        (Some(c), Some(m)) => format!("{action}: {c} ({m})"),
                        (Some(c), None) => format!("{action}: {c}"),
                        (None, Some(m)) => format!("{action}: failed ({m})"),
                        (None, None) => format!("{action}: failed"),
                    };
                    per.entry(t.serial.as_str()).or_default().push(line);
                    *failed.entry(t.serial.as_str()).or_default() |= !t.ok;
                }
                for (serial, lines) in per {
                    let bad = failed.get(serial).copied().unwrap_or(false);
                    let how = if bad { "failed" } else { "ran" };
                    let mut r = EventRecord::new(
                        EventKind::Schedule,
                        now.ms,
                        format!("Schedule {name} {how}"),
                    );
                    r.source = Some(rule.clone());
                    r.by = Some(by.clone().unwrap_or_else(|| format!("Schedule: {name}")));
                    r.lines = lines;
                    out.extend(self.push(serial, r, now));
                }
            }
            BusMessage::Event { serial, record } => {
                // Ours, already written, when it has an id; another service's to write
                // when it has none.
                if record.id != 0 {
                    return out;
                }
                let d = self.device(serial, now);
                if record.kind == EventKind::Firmware && record.fw.is_some() && record.fw == d.fw {
                    return out;
                }
                let mut r = record.clone();
                if r.t == 0 {
                    r.t = now.ms;
                }
                out.extend(self.push(serial, r, now));
            }
        }
        out
    }

    /// What a device's `Facts` say: a restart since the log last saw it, a firmware change.
    fn facts(
        &mut self,
        serial: &str,
        identify: bool,
        reset_info: Option<&ResetInfo>,
        fw: &str,
        now: Now,
    ) -> Vec<EventRecord> {
        let d = self.device(serial, now);
        d.ext_events = identify || reset_info.is_some();
        let mut out = Vec::new();
        // -- WP P2EVFIX -- F9: a firmware change, from the facts too (the inventory may be
        // off); `d.fw` keeps it one record with the inventory's `Event` (P2IN).
        if !fw.is_empty() && d.fw.as_deref() != Some(fw) {
            let text = match &d.fw {
                Some(old) => format!("Firmware {fw} (was {old})"),
                None => format!("Firmware {fw} (first seen by the hub)"),
            };
            let mut r = EventRecord::new(EventKind::Firmware, now.ms, text);
            r.fw = Some(fw.to_owned());
            out.push(r);
        }
        if let Some(ri) = reset_info {
            let restarted = d
                .uptime
                .is_none_or(|last| events::restarted(last, ri.uptime_s, now.ms));
            if restarted {
                let mut text = format!("The device restarted: {}", ri.reason.describe());
                if let Some(prev) = ri.prev_uptime_s {
                    text.push_str(&format!(" (the run before lasted {prev} s)"));
                }
                let mut r = EventRecord::new(EventKind::Reset, now.ms, text);
                r.source = Some(ri.reason.as_str().to_owned());
                r.uptime_s = Some(ri.uptime_s);
                if ri.hang_count > 0 {
                    r.lines = vec![format!("hang count {}", ri.hang_count)];
                }
                out.push(r);
            }
            // -- WP P2EVFIX -- F2: the health watch starts from this reading, so the next
            // `Health` with the same uptime is not the same restart again.
            let _ = d
                .watch
                .uptime(ri.uptime_s, Some(ri.hang_count), now.mono, now.ms);
            d.health_uptime = Some(ri.uptime_s);
        }
        out
    }

    /// What one health reading says: a temperature band crossed, a restart or a hang, a
    /// new clip on the latch.
    fn health(
        &mut self,
        serial: &str,
        clip_mask: u32,
        temp_c: Option<f32>,
        uptime_s: Option<u32>,
        hang_count: Option<u8>,
        now: Now,
    ) -> Vec<EventRecord> {
        let d = self.device(serial, now);
        let mut out = Vec::new();
        if let Some(c) = temp_c.filter(|c| c.is_finite()) {
            let band = match d.temp_band {
                None => band_of(c),
                Some(old) => next_band(old, c),
            };
            let old = d.temp_band.replace(band);
            if let Some(old) = old
                && old != band
            {
                let lo = (band as f32) * TEMP_BAND_C;
                let way = if band > old { "rising" } else { "falling" };
                let mut r = EventRecord::new(
                    EventKind::Temperature,
                    now.ms,
                    format!(
                        "Temperature {c:.1} °C ({lo:.0} to {:.0} °C, {way})",
                        lo + TEMP_BAND_C
                    ),
                );
                r.value = Some(f64::from(c));
                r.source = Some(way.into());
                out.push(r);
            }
        }
        // -- WP P2EVFIX -- F3: the uptime refreshes with the reset-info read (every 60 s),
        // `Health` every 2 s: only a new value is a reading.
        if let Some(u) = uptime_s
            && d.health_uptime != Some(u)
        {
            d.health_uptime = Some(u);
            if let Some(r) = d.watch.uptime(u, hang_count, now.mono, now.ms) {
                out.push(r);
            }
        }
        // The latch is "clipped since someone cleared": a bit that rose is a new clip.
        // Firmware that sends clip events records them from the events instead.
        if !d.ext_events {
            let first = d.clip_latch.is_none();
            let new = clip_mask & !d.clip_latch.unwrap_or(0);
            d.clip_latch = Some(clip_mask);
            for ch in channels(new) {
                let text = if first {
                    format!(
                        "Clip on channel {} (set before the hub looked)",
                        u16::from(ch).saturating_add(1)
                    )
                } else {
                    format!("Clip on channel {}", u16::from(ch).saturating_add(1))
                };
                let mut r = EventRecord::new(EventKind::Clip, now.ms, text);
                r.channel = Some(ch);
                r.source = Some("latch".into());
                out.push(r);
            }
        }
        out
    }

    /// Add `record` to `serial`'s log, coalescing it as the format does: a record with the
    /// key of one held back, within [`COALESCE_MS`] of it, replaces it and counts one more.
    /// One record is held per key (at most [`MAX_HELD`] per device), so two sweeps at once
    /// (a write and the device's own change it causes) each stay one record. A record
    /// that does not coalesce writes out everything held first. Returns what was written.
    fn push(&mut self, serial: &str, record: EventRecord, now: Now) -> Vec<EventRecord> {
        let d = self.device(serial, now);
        let mut out_first: Vec<EventRecord> = Vec::new();
        let mut last = None;
        if record.coalesce.is_some() {
            if let Some(p) = d.pending.iter_mut().find(|p| p.coalesce == record.coalesce)
                && record.t >= p.t
                && record.t.saturating_sub(p.t) < COALESCE_MS
            {
                let count = p.count.saturating_add(record.count);
                let t = p.t;
                *p = EventRecord { t, count, ..record };
                return Vec::new();
            }
            // The same key's window has passed: that one is done.
            if let Some(i) = d.pending.iter().position(|p| p.coalesce == record.coalesce) {
                out_first.push(d.pending.remove(i));
            }
            if d.pending.len() >= MAX_HELD {
                out_first.push(d.pending.remove(0));
            }
            d.pending.push(record);
        } else {
            out_first.append(&mut d.pending);
            last = Some(record);
        }
        let mut out = Vec::new();
        for p in out_first.into_iter().chain(last) {
            out.extend(self.write(serial, p));
        }
        out
    }

    /// Write out `serial`'s held-back records.
    pub(crate) fn flush(&mut self, serial: &str) -> Vec<EventRecord> {
        let held = self
            .devices
            .get_mut(serial)
            .map(|d| std::mem::take(&mut d.pending))
            .unwrap_or_default();
        held.into_iter()
            .filter_map(|p| self.write(serial, p))
            .collect()
    }

    /// Write out every held-back record whose window has passed at `now`, and a loss
    /// episode's count once it ended (nothing lost for [`EPISODE_QUIET_MS`]) or has lasted
    /// [`EPISODE_MAX_MS`].
    pub(crate) fn tick(&mut self, now: Now) -> Vec<EventRecord> {
        let mut due: Vec<(String, EventRecord)> = Vec::new();
        for (serial, d) in &mut self.devices {
            let (done, keep): (Vec<EventRecord>, Vec<EventRecord>) = std::mem::take(&mut d.pending)
                .into_iter()
                .partition(|p| now.ms.saturating_sub(p.t) >= COALESCE_MS);
            d.pending = keep;
            due.extend(done.into_iter().map(|p| (serial.clone(), p)));
        }
        let mut out = Vec::new();
        for (serial, p) in due {
            out.extend(self.write(&serial, p));
        }
        out.extend(self.write_lost(now, false));
        out
    }

    /// Everything held back, written (the hub stops).
    pub(crate) fn flush_all(&mut self, now: Now) -> Vec<EventRecord> {
        let serials: Vec<String> = self.devices.keys().cloned().collect();
        let mut out = Vec::new();
        for serial in serials {
            out.extend(self.flush(&serial));
        }
        out.extend(self.write_lost(now, true));
        out
    }

    /// `n` messages or records lost: added to the episode's count, which
    /// [`History::tick`] writes as one `events_dropped` when the episode ends.
    pub(crate) fn lost(&mut self, n: u64, now: Now) {
        self.lost_at(n, now.ms);
    }

    fn lost_at(&mut self, n: u64, ms: i64) {
        if n == 0 {
            return;
        }
        if self.lost == 0 {
            self.lost_since_ms = Some(ms);
        }
        self.lost = self.lost.saturating_add(n);
        self.lost_last_ms = ms;
    }

    /// `now_all`: the hub stops, write whatever is counted.
    fn write_lost(&mut self, now: Now, now_all: bool) -> Vec<EventRecord> {
        if self.lost == 0 {
            return Vec::new();
        }
        let since = self.lost_since_ms.unwrap_or(now.ms);
        let lasting = now.ms.saturating_sub(since) >= EPISODE_MAX_MS;
        let caught_up = now_all || now.ms.saturating_sub(self.lost_last_ms) >= EPISODE_QUIET_MS;
        let retry_wait = self
            .lost_tried_ms
            .is_some_and(|t| now.ms.saturating_sub(t) < LOST_RETRY_MS);
        if (!caught_up && !lasting) || retry_wait {
            return Vec::new();
        }
        let online: Vec<String> = self
            .devices
            .iter()
            .filter(|(_, d)| d.online)
            .map(|(s, _)| s.clone())
            .collect();
        let targets = if online.is_empty() {
            self.devices.keys().cloned().collect()
        } else {
            online
        };
        if targets.is_empty() {
            self.lost_tried_ms = Some(now.ms);
            return Vec::new();
        }
        let n = std::mem::take(&mut self.lost);
        let count = u32::try_from(n).unwrap_or(u32::MAX);
        let mut out = Vec::new();
        for serial in targets {
            let mut r = EventRecord::new(
                EventKind::EventsDropped,
                now.ms,
                format!("{n} hub events were lost before they reached the history"),
            );
            r.source = Some("hub".into());
            r.count = count;
            out.extend(self.push(&serial, r, now));
        }
        if out.iter().any(|r| r.kind == EventKind::EventsDropped) {
            // A failed write during the attempt is a new episode's first loss.
            self.lost_since_ms = (self.lost > 0).then_some(now.ms);
            self.lost_last_ms = now.ms;
            self.lost_tried_ms = None;
        } else {
            // Nothing could be written: try again later, with what came since.
            self.lost = self.lost.saturating_add(n);
            self.lost_tried_ms = Some(now.ms);
        }
        out
    }

    fn write(&mut self, serial: &str, record: EventRecord) -> Option<EventRecord> {
        self.notice_forget(serial);
        let record_t = record.t;
        match self.store.append(serial, record) {
            Ok(r) => {
                if let Some(d) = self.devices.get_mut(serial) {
                    d.wrote = true;
                    if r.fw.is_some() {
                        d.fw.clone_from(&r.fw);
                    }
                    if let Some(u) = r.uptime_s {
                        d.uptime = Some((r.t, u));
                    }
                }
                Some(r)
            }
            Err(e) => {
                log::warn!("hub: a history record for {serial} was lost: {e}");
                self.lost_at(1, record_t);
                None
            }
        }
    }

    /// Another writer's `forget` (the inventory's `inventory.forget` opens its own store on
    /// the folder) removed every file of `serial` that this history wrote: start its ids
    /// again at 1, as the forget meant, and forget what the log said of it. Also after the
    /// total cap removed a device's every file.
    fn notice_forget(&mut self, serial: &str) {
        let Some(d) = self.devices.get_mut(serial) else {
            return;
        };
        if !d.wrote {
            return;
        }
        let current = self
            .store
            .dir()
            .join(format!("{}.jsonl", events::file_stem(serial)));
        if current.exists() {
            return;
        }
        d.wrote = false;
        d.fw = None;
        d.uptime = None;
        // Nothing is left to remove: this only drops the store's next id.
        let _ = self.store.forget(serial);
    }

    /// Whether the history knows `serial`: a log in the folder, or seen this run.
    fn knows(&self, serial: &str) -> bool {
        self.devices.contains_key(serial) || self.store.devices().iter().any(|s| s == serial)
    }

    /// Every record of `serial`, oldest first, after writing out its held-back one.
    fn all(&mut self, serial: &str, written: &mut Vec<EventRecord>) -> Vec<EventRecord> {
        written.extend(self.flush(serial));
        self.store.read(serial, 0, usize::MAX).records
    }

    /// `event.list`'s answer body.
    pub(crate) fn list(
        &mut self,
        serial: &str,
        after_id: u64,
        limit: usize,
        written: &mut Vec<EventRecord>,
    ) -> Value {
        let all = self.all(serial, written);
        let newest_id = all.last().map_or(0, |r| r.id);
        let mut after: Vec<EventRecord> = all.into_iter().filter(|r| r.id > after_id).collect();
        let more = if after_id == 0 {
            let drop = after.len().saturating_sub(limit);
            after.drain(..drop);
            false
        } else {
            let more = after.len() > limit;
            after.truncate(limit);
            more
        };
        json!({"serial": serial, "records": after, "more": more, "newest_id": newest_id})
    }

    /// `event.export`'s answer body.
    pub(crate) fn export(
        &mut self,
        serial: &str,
        after_id: u64,
        written: &mut Vec<EventRecord>,
    ) -> Value {
        let all = self.all(serial, written);
        let newest_id = all.last().map_or(0, |r| r.id);
        let mut text = String::new();
        let mut last = None;
        let mut rest = all.iter().filter(|r| r.id > after_id).peekable();
        while let Some(r) = rest.peek() {
            let mut line = serde_json::to_string(r).unwrap_or_default();
            line.push('\n');
            if !text.is_empty() && text.len().saturating_add(line.len()) > EXPORT_PAGE_BYTES {
                break;
            }
            text.push_str(&line);
            last = Some(r.id);
            rest.next();
        }
        let next = if rest.peek().is_some() { last } else { None };
        json!({"serial": serial, "text": text, "next_after_id": next, "newest_id": newest_id})
    }
}

/// The 5 °C band a first reading is in.
fn band_of(c: f32) -> i32 {
    (c / TEMP_BAND_C).floor() as i32
}

/// The band after `old` for a reading of `c`: left only `TEMP_HYSTERESIS_C` past an edge.
fn next_band(old: i32, c: f32) -> i32 {
    let lo = (old as f32) * TEMP_BAND_C;
    let hi = lo + TEMP_BAND_C;
    if c >= hi + TEMP_HYSTERESIS_C || c < lo - TEMP_HYSTERESIS_C {
        band_of(c)
    } else {
        old
    }
}

/// The channels whose bit is set, ascending.
fn channels(mask: u32) -> impl Iterator<Item = u8> {
    (0u8..32).filter(move |&ch| mask & 1u32.checked_shl(u32::from(ch)).unwrap_or(0) != 0)
}

/// The records for one relayed notification. An extension event is read as one only on
/// firmware with the extension (`ext`).
fn notification_records(
    packet: &[u8],
    session: SessionId,
    by: Option<&str>,
    ext: bool,
    t: i64,
) -> Vec<EventRecord> {
    let event = match notify::decode(packet) {
        Ok((_, event)) => event,
        Err(e) => {
            let mut r = EventRecord::new(
                EventKind::Notification,
                t,
                format!("A notification the hub could not read ({e})"),
            );
            r.coalesce = Some("undecodable".into());
            return vec![attributed(r, session, by)];
        }
    };
    let (text, source, key): (String, Option<Source>, Option<String>) = match &event {
        Event::Idle | Event::MasterVolume(_) => return Vec::new(),
        Event::ParamChanged { offset, source, .. } => (
            format!("{} changed", field_name(*offset)),
            Some(*source),
            Some(format!("p{offset}:{source:?}:{session}")),
        ),
        Event::BulkInvalidated { source } => (
            "The whole settings image changed".into(),
            Some(*source),
            None,
        ),
        Event::PresetLoaded { slot } => (format!("Preset {slot} loaded"), None, None),
        Event::InputFormat { channels } => (format!("Input channels: {channels}"), None, None),
        Event::CsAux {
            slot,
            state,
            level_q8,
            source,
        } => (
            format!(
                "Control surface output {slot}: {} ({:.1} %)",
                if *state != 0 { "on" } else { "off" },
                f64::from(*level_q8) / 256.0
            ),
            Some(*source),
            Some(format!("aux{slot}:{source:?}:{session}")),
        ),
        Event::Unknown { id, bytes } => {
            // On extension firmware every extension event counts as announced: the
            // device's `EXT_NOTIFY_FEATURES` is not on the bus, and 0xE0-0xE3 are the
            // extension's on that firmware.
            let features = ext.then_some(u32::MAX);
            match decode_ext_event(features, *id, bytes) {
                Some(ExtEvent::Clip { mask }) => {
                    return channels(mask)
                        .map(|ch| {
                            let mut r = EventRecord::new(
                                EventKind::Clip,
                                t,
                                format!("Clip on channel {}", u16::from(ch).saturating_add(1)),
                            );
                            r.channel = Some(ch);
                            r.source = Some("event".into());
                            r
                        })
                        .collect();
                }
                Some(ExtEvent::Reset { reason }) => {
                    let mut r = EventRecord::new(
                        EventKind::Reset,
                        t,
                        format!("The device restarted ({})", reason.as_str()),
                    );
                    r.source = Some(reason.as_str().into());
                    return vec![r];
                }
                Some(e) => return alink_record(&e, t).into_iter().collect(),
                None => (
                    format!("Event 0x{id:02X} ({} bytes)", bytes.len()),
                    None,
                    None,
                ),
            }
        }
        other => (event_text(other), None, None),
    };
    let mut r = EventRecord::new(EventKind::Notification, t, text);
    r.source = source.map(source_label);
    r.coalesce = key;
    vec![attributed(r, session, by)]
}

fn attributed(mut r: EventRecord, session: SessionId, by: Option<&str>) -> EventRecord {
    r.session = (session != 0).then_some(u32::from(session));
    r.by = by.map(str::to_owned);
    r
}

/// The bulk image's field at `offset`: its section, and where in it.
fn field_name(offset: u16) -> String {
    let off = usize::from(offset);
    SECTIONS
        .iter()
        .find(|(_, start, len)| off >= *start && off < start.saturating_add(*len))
        .map_or_else(
            || format!("offset 0x{off:04X}"),
            |(name, start, _)| match off.saturating_sub(*start) {
                0 => (*name).to_owned(),
                rel => format!("{name}+0x{rel:X}"),
            },
        )
}

/// The rest of the events, plainly.
fn event_text(event: &Event) -> String {
    match event {
        Event::SiggenState { state, .. } => format!("Signal generator state {state}"),
        Event::AdatState {
            enabled, active, ..
        } => format!(
            "ADAT output {}, {}",
            if *enabled { "on" } else { "off" },
            if *active { "active" } else { "idle" }
        ),
        Event::I2sSlaveState { state, rate_hz } => {
            format!("I2S input state {state} ({rate_hz} Hz)")
        }
        Event::IrLearn { state, code, .. } => format!("IR learn state {state} (code 0x{code:08X})"),
        Event::AdatInputState { state, rate_hz, .. } => {
            format!("ADAT input state {state} ({rate_hz} Hz)")
        }
        other => format!("{other:?}"),
    }
}

/// The `ParamSource` tag as notify.h spells it, short (as the client's monitor writes it).
fn source_label(source: Source) -> String {
    match source {
        Source::Unknown => "?".into(),
        Source::HostSet => "HOST".into(),
        Source::BulkSet => "BULK".into(),
        Source::Preset => "PRESET".into(),
        Source::Factory => "FACTORY".into(),
        Source::Gpio => "GPIO".into(),
        Source::Internal => "INTERNAL".into(),
        Source::Uac1 => "UAC1".into(),
        Source::Uart => "UART".into(),
        Source::I2c => "I2C".into(),
        Source::Midi => "MIDI".into(),
        Source::Other(raw) => format!("src=0x{raw:02X}"),
    }
}

// ── The messages ────────────────────────────────────────────────────────────

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn ok(id: Option<&Value>, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

/// A device named by `serial`, `handle` or both (hub-api.md 1.3), as its serial.
fn addressed(hub: &HubHandle, msg: &Value) -> Result<String, (&'static str, &'static str)> {
    let serial = match msg.get("serial") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && s.len() <= events::store::MAX_SERIAL_BYTES => {
            Some(s.clone())
        }
        Some(_) => return Err(("bad_request", "serial: a device's serial")),
    };
    let handle = match msg.get("handle") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64().and_then(|h| u8::try_from(h).ok()) {
            Some(h) => Some(h),
            None => return Err(("bad_request", "handle: a device's handle")),
        },
    };
    let by_handle = match handle {
        None => None,
        Some(h) => match hub.devices().into_iter().find(|d| d.handle == h) {
            Some(d) => Some(d.identity.serial),
            None => return Err(("no_device", "no device has that handle")),
        },
    };
    match (serial, by_handle) {
        (Some(s), Some(h)) if s != h => Err(("bad_request", "serial and handle name two devices")),
        (Some(s), _) | (None, Some(s)) => Ok(s),
        (None, None) => Err(("bad_request", "serial: which device")),
    }
}

fn after_id(msg: &Value) -> Result<u64, (&'static str, &'static str)> {
    match msg.get("after_id") {
        None | Some(Value::Null) => Ok(0),
        Some(v) => v
            .as_u64()
            .ok_or(("bad_request", "after_id: a record's id, or 0")),
    }
}

fn limit(msg: &Value) -> Result<usize, (&'static str, &'static str)> {
    match msg.get("limit") {
        None | Some(Value::Null) => Ok(PAGE_DEFAULT),
        Some(v) => v
            .as_u64()
            .filter(|n| *n > 0)
            .map(|n| usize::try_from(n).unwrap_or(PAGE_MAX).min(PAGE_MAX))
            .ok_or(("bad_request", "limit: how many records, 1 to 500")),
    }
}

// ── The service ─────────────────────────────────────────────────────────────

/// A session watching.
struct Watcher {
    session: SessionId,
    outbox: Weak<Outbox>,
    serial: Option<String>,
}

/// What the service and its thread share.
struct Shared {
    history: Mutex<History>,
    watchers: Mutex<Vec<Watcher>>,
    hub_id: OnceLock<String>,
    bus: Arc<Bus>,
    clock: Arc<dyn HubClock>,
}

impl Shared {
    fn now(&self) -> Now {
        Now {
            ms: self.clock.unix_ms(),
            mono: self.clock.now(),
        }
    }

    /// Tell the bus and the watchers about records just written. Called with the history
    /// held, so they hear them in id order; the bus and the outboxes never block.
    fn announce(&self, written: Vec<EventRecord>) {
        if written.is_empty() {
            return;
        }
        let mut watchers = lock(&self.watchers);
        watchers.retain(|w| w.outbox.upgrade().is_some_and(|o| !o.is_closed()));
        let hub = self.hub_id.get().map_or("", String::as_str);
        for r in written {
            for w in watchers.iter() {
                if w.serial.as_ref().is_some_and(|s| *s != r.serial) {
                    continue;
                }
                if let Some(o) = w.outbox.upgrade() {
                    let event = json!({"t": "event.added", "v": VERSION, "hub": hub,
                        "serial": r.serial, "record": r});
                    o.push_reliable(Outbound::Event(event.to_string()));
                }
            }
            self.bus.publish(BusMessage::Event {
                serial: r.serial.clone(),
                record: r,
            });
        }
    }

    /// One turn of the thread: the bus's drops, a message, the coalescing windows.
    fn step(&self, msg: Option<&BusMessage>, dropped: u64) {
        let now = self.now();
        let mut h = lock(&self.history);
        let mut written = Vec::new();
        h.lost(dropped, now);
        if let Some(m) = msg {
            written.extend(h.on_message(m, now));
        }
        written.extend(h.tick(now));
        self.announce(written);
    }

    fn run(&self, sub: &Subscription, stop: &AtomicBool, hold: Option<&AtomicBool>) {
        while !stop.load(Ordering::SeqCst) {
            if hold.is_some_and(|h| h.load(Ordering::SeqCst)) {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let msg = sub.recv_timeout(POLL);
            self.step(msg.as_deref(), sub.take_dropped());
        }
        // What came before the stop, then everything held back.
        for m in sub.drain() {
            self.step(Some(m.as_ref()), sub.take_dropped());
        }
        let now = self.now();
        let mut h = lock(&self.history);
        let written = h.flush_all(now);
        self.announce(written);
    }

    fn handle(&self, hub: &HubHandle, session: &Arc<Session>, msg: &Value) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if !matches!(t, "event.list" | "event.watch" | "event.export") {
            return None;
        }
        let _ = self.hub_id.set(hub.hub_id().to_owned());
        let id = msg.get("id");
        let reply = match t {
            "event.watch" => self.watch(session, msg),
            "event.export" if session.role() < Role::Admin => {
                Err(("denied", "event.export is for an admin"))
            }
            _ => self.read(hub, t, msg),
        };
        Some(match reply {
            Ok(body) => ok(id, body),
            Err((code, text)) => err(id, code, text),
        })
    }

    fn read(
        &self,
        hub: &HubHandle,
        t: &str,
        msg: &Value,
    ) -> Result<Value, (&'static str, &'static str)> {
        let serial = addressed(hub, msg)?;
        let after = after_id(msg)?;
        let limit = if t == "event.list" { limit(msg)? } else { 0 };
        let mut h = lock(&self.history);
        if !h.knows(&serial) {
            return Err(("not_found", "the hub has no history of that device"));
        }
        let mut written = Vec::new();
        let body = if t == "event.list" {
            h.list(&serial, after, limit, &mut written)
        } else {
            h.export(&serial, after, &mut written)
        };
        self.announce(written);
        Ok(body)
    }

    fn watch(
        &self,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Result<Value, (&'static str, &'static str)> {
        let Some(on) = msg.get("on").and_then(Value::as_bool) else {
            return Err(("bad_request", "on: true or false"));
        };
        let serial = match msg.get("serial") {
            None | Some(Value::Null) => None,
            Some(Value::String(s))
                if !s.is_empty() && s.len() <= events::store::MAX_SERIAL_BYTES =>
            {
                Some(s.clone())
            }
            Some(_) => return Err(("bad_request", "serial: a device's serial")),
        };
        let mut watchers = lock(&self.watchers);
        watchers.retain(|w| {
            w.session != session.id() && w.outbox.upgrade().is_some_and(|o| !o.is_closed())
        });
        if on {
            if watchers.len() >= MAX_WATCHERS {
                return Err(("conflict", "too many sessions are watching"));
            }
            watchers.push(Watcher {
                session: session.id(),
                outbox: Arc::downgrade(session.outbox()),
                serial: serial.clone(),
            });
        }
        Ok(json!({"on": on, "serial": serial}))
    }
}

/// What the history takes from the bus: everything but written records (its own echoes,
/// and any other writer's), which it has already.
fn wants(m: &BusMessage) -> bool {
    !matches!(m, BusMessage::Event { record, .. } if record.id != 0)
}

/// The history as a hub service.
pub(crate) struct HistoryService {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl HistoryService {
    fn start(
        config: &HistoryConfig,
        dir: PathBuf,
        clock: &Arc<dyn HubClock>,
        bus: &Arc<Bus>,
    ) -> Result<Arc<dyn HubService>, String> {
        let history = History::open(dir, config).map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared {
            history: Mutex::new(history),
            watchers: Mutex::new(Vec::new()),
            hub_id: OnceLock::new(),
            bus: Arc::clone(bus),
            clock: Arc::clone(clock),
        });
        // -- WP P2EVFIX -- F1: its own echoes (records with an id) never take room in its
        // queue, nor count as dropped.
        let sub = bus.subscribe_filtered("history", config.bus_capacity, wants);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let hold = config.test_hold.clone();
            std::thread::Builder::new()
                .name("hub-history".into())
                .spawn(move || shared.run(&sub, &stop, hold.as_deref()))
                .map_err(|e| format!("its thread: {e}"))?
        };
        Ok(Arc::new(Self {
            shared,
            stop,
            thread: Mutex::new(Some(thread)),
        }))
    }
}

impl HubService for HistoryService {
    fn name(&self) -> &'static str {
        "history"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        self.shared.handle(hub, session, msg)
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = lock(&self.thread).take()
            && t.join().is_err()
        {
            log::error!("hub: the history's thread panicked");
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2ev;
