//! The hub's system model for the audio link (WP SW2, SYS2 section 7): the
//! installation ([`model`]), the alignment engine ([`align`]), scenes applied at
//! a frame ([`scene`]) and alink alarms ([`alarms`]), kept in the hub's store and
//! served over Link (`system.*`, `docs/integrator/audio-link.md`).
//!
//! The pure parts take what the hub read and say what to write; `run` reads and
//! writes the devices through their workers as the hub's own session, so a
//! client's lock, the policy and BULKFIX's guard apply as to any write. Those
//! calls block on device transfers: they run on a connection's reader, as
//! `device.snapshot` does, never on a hub thread.
//!
//! Lock order: [`SystemShared`]'s lock is a leaf. It is never held across a
//! transfer, a save, a relay call or the hub's state lock.

pub mod alarms;
pub mod align;
// -- WP ATGUARD -- the guard of a system apply (`system.guard.*`)
pub mod guard;
// -- WP ATHUB -- the `system.measure.*` family
pub mod measure;
pub mod model;
mod run;
pub mod scene;

pub(crate) use run::learn_events;
pub use run::{AlignReport, SceneReport, SystemRefused, SystemStatus, WriteResult};
// -- WP AUSB1 --
pub use run::USB_HELD_BACK;
// -- WP AUSB2 --
pub use run::{USB_LEAD_CAPPED, USB_LEAD_EXCEEDED};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use dspi_proto::ext::alink::{AlinkEvent, alink_event};
use dspi_proto::ext::alink_usb::{self as usb, UsbEventReason};
use serde_json::json;

use crate::error::HubError;
use crate::relay::Relay;
use alarms::{Alarm, AlarmBook};
use align::{Alignment, Observed};
use model::SystemModel;

/// Why `system.set` did not take a model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelRefused {
    /// It does not pass [`SystemModel::validate`].
    #[error("{0}")]
    Invalid(model::Invalid),
    /// The store could not keep it.
    #[error("not saved: {0}")]
    NotSaved(HubError),
    // -- fixup sw3x --
    /// `system.set` named the revision it was edited from, and the hub has a
    /// newer one: someone else changed the installation meanwhile.
    #[error("the installation changed on the hub (revision {current}); read it again")]
    Stale {
        /// The hub's revision now.
        current: u64,
    },
}

/// Where the model persists: the hub's store (`hub-store.json`, `system`).
pub trait SystemStore: Send + Sync {
    /// The model kept from last time, if one was kept and is valid.
    fn load(&self) -> Option<SystemModel>;
    /// Replace the model kept.
    fn save(&self, model: &SystemModel) -> Result<(), HubError>;
    // -- WP AUSB2 --
    /// The USB-direct start offsets the hub last wrote into the units' output
    /// delays, by unit id, if they were kept (`system_usb_offsets`).
    fn load_usb_offsets(&self) -> Option<BTreeMap<String, i64>> {
        None
    }
    /// Keep them, so a restarted hub still knows what it wrote.
    fn save_usb_offsets(&self, _offsets: &BTreeMap<String, i64>) -> Result<(), HubError> {
        Ok(())
    }
}

#[derive(Default)]
struct State {
    model: SystemModel,
    /// Bumped by every accepted `system.set`.
    revision: u64,
    alarms: AlarmBook,
    /// The last reads, by serial.
    observed: BTreeMap<String, Observed>,
    // -- fixup sw2x (F5) --
    /// Whether each device announces 0xE1 (`EXT_NOTIFY_FEATURES` bit 2), by
    /// serial: learned when it attaches and at each read, so its events count
    /// whatever handle it has now.
    events: BTreeMap<String, bool>,
    alignment: Option<Alignment>,
    /// Units the hub has just re-anchored itself (a new `EXT_ALINK_PHASE`), by
    /// serial: their `REANCHOR` event before this is no alarm.
    reanchoring: BTreeMap<String, Instant>,
    // -- WP FIXM1 --
    /// The USB-direct start offsets the hub last wrote into the units' output
    /// delays, by unit id (none: 0); `None` until the first alignment written
    /// in this run, so nothing is judged before it. -- WP AUSB2 -- Kept in
    /// the store: a restarted hub starts from what it wrote before.
    usb_offsets_written: Option<BTreeMap<String, i64>>,
    // -- WP AUSB2 --
    /// Whether each device announces 0xE3 (`EXT_NOTIFY_FEATURES` bit 4), by
    /// serial.
    usb_events: BTreeMap<String, bool>,
    /// USB-direct units (serials) that refused "play unaligned": firmware
    /// before the fork's WP USB3b. Forgotten when the unit attaches again.
    play_unaligned_refused: BTreeSet<String>,
    /// When an automatic write that failed may be tried again, by key
    /// (`realign:<unit id>`, `lock:<serial>`): no retry storm.
    retry_after: BTreeMap<String, Instant>,
    // -- fixup ausb2 (F2) --
    /// USB-direct units (serials) that refused their settings, and why (D
    /// outside its range at this rate): the issue `usb_settings_refused`.
    settings_refused: BTreeMap<String, String>,
}

/// The system's state, shared by the hub's handles and the relay's tap.
pub(crate) struct SystemShared {
    state: Mutex<State>,
    store: Option<Arc<dyn SystemStore>>,
    /// Keeps saves in the order of the changes. Above `state`.
    saving: Mutex<()>,
    // -- WP AUSB2 --
    /// The hub's own poll of the model's USB-direct units, ms between two.
    poll_ms: AtomicU64,
    // -- fixup ausb2 (F6) --
    /// One writer per unit (by serial) for the USB-direct settings, delays
    /// and scene ops: the hub's poll, an alignment and a scene take turns. A
    /// leaf beside `state`, never taken while `state` is held.
    unit_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    /// Bumped by every alignment that writes: a poll whose reads came before
    /// one does not write from them.
    aligns: AtomicU64,
}

impl std::fmt::Debug for SystemShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SystemShared")
    }
}

/// How long after the hub writes a unit's phase its re-anchor is expected.
const REANCHOR_EXPECTED: Duration = Duration::from_secs(2);

// -- WP AUSB2 --
/// How often the hub reads the model's USB-direct units by itself (when the
/// model has any): it keeps them aligned when asked, writes their lock-error
/// action, and raises their alarms without a client asking for a status.
pub const SYSTEM_POLL: Duration = Duration::from_secs(5);

/// Unix seconds now.
pub(crate) fn now() -> u64 {
    crate::store::unix_seconds(SystemTime::now())
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `system.alarm` event.
pub(crate) fn alarm_event(alarm: &Alarm) -> String {
    json!({"t": "system.alarm", "alarm": alarm}).to_string()
}

/// The unit or fan-out of the model with this serial, by id. A fan-out's own
/// loss silences every unit behind it, so it alarms under its own id.
fn element_of(model: &SystemModel, serial: &str) -> Option<String> {
    model
        .units
        .iter()
        .find(|u| u.serial == serial)
        .map(|u| u.id.clone())
        .or_else(|| {
            model
                .fan_outs
                .iter()
                .find(|f| f.serial.as_deref() == Some(serial))
                .map(|f| f.id.clone())
        })
}

impl SystemShared {
    /// The model from `store`, or an empty one.
    pub(crate) fn new(store: Option<Arc<dyn SystemStore>>) -> Self {
        let model = store.as_ref().and_then(|s| s.load()).unwrap_or_default();
        // -- WP AUSB2 -- what the hub wrote before a restart
        let usb_offsets_written = store.as_ref().and_then(|s| s.load_usb_offsets());
        Self {
            state: Mutex::new(State {
                model,
                usb_offsets_written,
                ..State::default()
            }),
            store,
            saving: Mutex::new(()),
            poll_ms: AtomicU64::new(u64::try_from(SYSTEM_POLL.as_millis()).unwrap_or(u64::MAX)),
            unit_locks: Mutex::new(BTreeMap::new()),
            aligns: AtomicU64::new(0),
        }
    }

    // -- fixup ausb2 (F6) --
    /// Alignments written so far in this run.
    pub(crate) fn align_generation(&self) -> u64 {
        self.aligns.load(Ordering::SeqCst)
    }

    // -- fixup ausb2 (F6) --
    /// An alignment is writing.
    pub(crate) fn bump_align_generation(&self) {
        self.aligns.fetch_add(1, Ordering::SeqCst);
    }

    // -- fixup ausb2 (F6) --
    /// Unit `serial`'s write lock. Several are always taken in serial order.
    pub(crate) fn unit_lock(&self, serial: &str) -> Arc<Mutex<()>> {
        Arc::clone(
            guard(&self.unit_locks)
                .entry(serial.to_owned())
                .or_default(),
        )
    }

    // -- fixup ausb2 (F2) --
    /// Unit `serial` refused its settings (`Some(why)`), or took them.
    pub(crate) fn settings_refused(&self, serial: &str, why: Option<String>) {
        let mut s = self.lock();
        match why {
            Some(w) => {
                s.settings_refused.insert(serial.to_owned(), w);
            }
            None => {
                s.settings_refused.remove(serial);
            }
        }
    }

    // -- fixup ausb2 (F2) --
    /// Why unit `serial` refused its settings at the last write, if it did.
    pub(crate) fn refused_settings(&self, serial: &str) -> Option<String> {
        self.lock().settings_refused.get(serial).cloned()
    }

    // -- fixup ausb2 (F5) --
    /// Record that the hub put a unit's setting back to the installation's
    /// (alarm `usb_setting_restored`); the changes to broadcast.
    pub(crate) fn usb_setting_restored(&self, unit: &str, message: String) -> Vec<Alarm> {
        self.lock()
            .alarms
            .usb_setting_restored(unit, message, now())
    }

    // -- WP AUSB2 --
    /// How long the hub's own poll waits between two reads.
    pub(crate) fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_ms.load(Ordering::Relaxed))
    }

    // -- WP AUSB2 --
    /// Change [`Self::poll_interval`] (at least 100 ms).
    pub(crate) fn set_poll_interval(&self, every: Duration) {
        let ms = u64::try_from(every.as_millis())
            .unwrap_or(u64::MAX)
            .max(100);
        self.poll_ms.store(ms, Ordering::Relaxed);
    }

    // -- WP AUSB2 --
    /// Whether an automatic write under `key` may be tried now; when it may,
    /// the next try is held off for [`alarms::REALIGN_RETRY_S`].
    pub(crate) fn may_retry(&self, key: &str) -> bool {
        let mut s = self.lock();
        let now = Instant::now();
        if s.retry_after.get(key).is_some_and(|t| now < *t) {
            return false;
        }
        if let Some(t) = now.checked_add(Duration::from_secs(alarms::REALIGN_RETRY_S)) {
            s.retry_after.insert(key.to_owned(), t);
        }
        true
    }

    // -- WP AUSB2 --
    /// The automatic write under `key` worked: the next may come at once.
    pub(crate) fn retry_done(&self, key: &str) {
        self.lock().retry_after.remove(key);
    }

    // -- WP AUSB2 --
    /// Unit `serial` refused "play unaligned" (`true`), or took it.
    pub(crate) fn play_unaligned_refused(&self, serial: &str, refused: bool) {
        let mut s = self.lock();
        if refused {
            s.play_unaligned_refused.insert(serial.to_owned());
        } else {
            s.play_unaligned_refused.remove(serial);
        }
    }

    // -- WP AUSB2 --
    /// Whether unit `serial` refused "play unaligned" in this run.
    pub(crate) fn refuses_play_unaligned(&self, serial: &str) -> bool {
        self.lock().play_unaligned_refused.contains(serial)
    }

    // -- WP AUSB2 --
    /// A device attached: what was learned of its firmware's lock-error
    /// setting and of failed writes to it no longer holds.
    pub(crate) fn attached(&self, serial: &str) {
        let mut s = self.lock();
        s.play_unaligned_refused.remove(serial);
        s.settings_refused.remove(serial);
        s.retry_after.remove(&format!("lock:{serial}"));
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        guard(&self.state)
    }

    /// The model and its revision.
    pub(crate) fn model(&self) -> (SystemModel, u64) {
        let s = self.lock();
        (s.model.clone(), s.revision)
    }

    /// Take a new model: checked, saved, then kept. The revision it got. A
    /// model that is refused or cannot be saved changes nothing.
    #[cfg(test)]
    pub(crate) fn set_model(&self, model: SystemModel) -> Result<u64, ModelRefused> {
        self.set_model_at(model, None)
    }

    // -- fixup sw3x --
    /// [`Self::set_model`], refused as [`ModelRefused::Stale`] when `expected`
    /// is given and is not the revision now. Checked under the saving lock,
    /// which every change of the revision holds.
    pub(crate) fn set_model_at(
        &self,
        model: SystemModel,
        expected: Option<u64>,
    ) -> Result<u64, ModelRefused> {
        model.validate().map_err(ModelRefused::Invalid)?;
        let _in_order = guard(&self.saving);
        if let Some(expected) = expected {
            let current = self.lock().revision;
            if current != expected {
                return Err(ModelRefused::Stale { current });
            }
        }
        if let Some(store) = &self.store {
            store.save(&model).map_err(ModelRefused::NotSaved)?;
        }
        let mut s = self.lock();
        s.model = model;
        s.revision = s.revision.saturating_add(1);
        s.alignment = None;
        Ok(s.revision)
    }

    pub(crate) fn alarms(&self) -> Vec<Alarm> {
        self.lock().alarms.list()
    }

    /// Acknowledge alarm `id`; the change to broadcast.
    pub(crate) fn acknowledge(&self, id: u64) -> Option<Alarm> {
        self.lock().alarms.acknowledge(id, now())
    }

    // -- fixup sw2x (F5) --
    /// Whether `serial` is an element of the model (its events matter).
    pub(crate) fn watches(&self, serial: &str) -> bool {
        element_of(&self.lock().model, serial).is_some()
    }

    /// What `serial`'s `EXT_NOTIFY_FEATURES` says about 0xE1 and -- WP AUSB2
    /// -- 0xE3.
    pub(crate) fn learned_events(&self, serial: &str, announces: bool, usb_events: bool) {
        let mut s = self.lock();
        s.events.insert(serial.to_owned(), announces);
        s.usb_events.insert(serial.to_owned(), usb_events);
    }

    // -- fixup sw2x (F1) --
    /// The system mute of sender `id` as read or written; the alarm changes.
    pub(crate) fn system_mute_seen(&self, id: &str, muted: bool) -> Vec<Alarm> {
        self.lock().alarms.system_mute(id, muted, now())
    }

    /// Keep what a read found; the alarm changes to broadcast.
    pub(crate) fn observed(
        &self,
        observed: BTreeMap<String, Observed>,
        alignment: Alignment,
    ) -> Vec<Alarm> {
        let mut s = self.lock();
        let units: Vec<(String, String)> = s
            .model
            .units
            .iter()
            .map(|u| (u.id.clone(), u.serial.clone()))
            .collect();
        let t = now();
        let mut changes = Vec::new();
        for (id, serial) in units {
            changes.extend(s.alarms.observe(&id, observed.get(&serial), t));
        }
        // -- WP AUSB2 -- USB-direct units in lock error
        let usb_units: Vec<(String, String, String)> = s
            .model
            .units
            .iter()
            .filter(|u| u.usb_direct())
            .map(|u| {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                (u.id.clone(), u.serial.clone(), name.clone())
            })
            .collect();
        for (id, serial, name) in usb_units {
            let o = observed.get(&serial);
            let error = match o.filter(|o| o.online).map(|o| o.usb.as_ref()) {
                // Offline: `unit_offline` says it.
                None => Some(None),
                // Attached, not read: as it was.
                Some(None) => None,
                Some(Some(u)) => Some(u.lock_error.then(|| {
                    (
                        align::lock_error_message(
                            &name,
                            &u.on_lock_error,
                            u.lock_error_cause.as_deref(),
                        ),
                        u.on_lock_error != "play_unaligned",
                    )
                })),
            };
            if let Some(error) = error {
                changes.extend(s.alarms.usb_lock_error(&id, error, t));
            }
        }
        let senders: Vec<(String, Option<bool>)> = s
            .model
            .senders
            .iter()
            .map(|x| {
                let muted = x
                    .serial
                    .as_ref()
                    .and_then(|serial| observed.get(serial))
                    .and_then(|o| o.system_mute);
                (x.id.clone(), muted)
            })
            .collect();
        for (id, muted) in senders {
            if let Some(m) = muted {
                changes.extend(s.alarms.system_mute(&id, m, t));
            }
        }
        for (serial, o) in &observed {
            if o.online {
                s.events.insert(serial.clone(), o.state_events);
            }
        }
        s.observed = observed;
        s.alignment = Some(alignment);
        changes
    }

    // -- fixup hwlk1f (F4) --
    /// What `serial` reported at the last read, if it was read.
    pub(crate) fn last_observed(&self, serial: &str) -> Option<Observed> {
        self.lock().observed.get(serial).cloned()
    }

    // -- WP HWLK1F --
    /// Whether `serial` was last read with a firmware without the audio link.
    pub(crate) fn last_seen_without_alink(&self, serial: &str) -> bool {
        self.lock()
            .observed
            .get(serial)
            .is_some_and(Observed::without_alink)
    }

    // -- WP FIXM1 --
    /// The USB-direct units whose start offset (`now`, by unit id) is no
    /// longer the one the hub wrote; none before the first alignment written.
    pub(crate) fn usb_offsets_moved(&self, now: &BTreeMap<String, i64>) -> Vec<String> {
        self.lock()
            .usb_offsets_written
            .as_ref()
            .map(|w| align::usb_offsets_moved(w, now))
            .unwrap_or_default()
    }

    // -- WP FIXM1 --
    /// The hub wrote unit `id`'s output delays with this start offset (none:
    /// 0). -- WP AUSB2 -- Saved in the store when it changed.
    pub(crate) fn usb_offset_written(&self, id: &str, offset: Option<i64>) {
        let _in_order = guard(&self.saving);
        let save = {
            let mut s = self.lock();
            let first = s.usb_offsets_written.is_none();
            let w = s.usb_offsets_written.get_or_insert_with(BTreeMap::new);
            let before = w.clone();
            match offset {
                Some(o) => {
                    w.insert(id.to_owned(), o);
                }
                None => {
                    w.remove(id);
                }
            }
            (first || before != *w).then(|| w.clone())
        };
        if let (Some(w), Some(store)) = (save, &self.store)
            && let Err(e) = store.save_usb_offsets(&w)
        {
            log::warn!("hub: the USB-direct start offsets written are not kept: {e}");
        }
    }

    // -- WP AUSB2 --
    /// Record "Keep USB-direct units aligned" failing on some units (`Some`:
    /// the sentence) or not (`None`); the changes to broadcast.
    pub(crate) fn usb_realign_failed(&self, failed: Option<String>) -> Vec<Alarm> {
        self.lock().alarms.usb_realign_failed(failed, now())
    }

    // -- WP FIXM1 --
    /// Record an automatic re-align (alarm `usb_realigned`); the changes to
    /// broadcast.
    pub(crate) fn usb_realigned(&self, message: String) -> Vec<Alarm> {
        self.lock().alarms.usb_realigned(message, now())
    }

    /// The hub is about to write unit `serial`'s phase: its re-anchor follows.
    pub(crate) fn expect_reanchor(&self, serial: &str) {
        if let Some(until) = Instant::now().checked_add(REANCHOR_EXPECTED) {
            self.lock().reanchoring.insert(serial.to_owned(), until);
        }
    }

    /// Whether a packet is one [`Self::on_packet`] looks at (0xE1; -- WP
    /// AUSB2 -- and 0xE3), so the caller fetches the device's serial only for
    /// those.
    pub(crate) fn wants(packet: &[u8]) -> bool {
        matches!(
            packet.get(1),
            Some(
                &dspi_proto::ext::alink::NOTIFY_EVT_ALINK_STATE
                    | &usb::NOTIFY_EVT_ALINK_USB
                    | &dspi_proto::ext::timebase::NOTIFY_EVT_TIME_BASE // -- WP ATHUBM --
            )
        )
    }

    // -- WP ATHUBM --
    /// An 0xE4 from unit `serial` of the model that announces it (its last
    /// read saw `EXT_NOTIFY_FEATURES` bit 5): the alarms of its transport
    /// ([`AlarmBook::time_base_event`]). The changes.
    fn on_time_base_event(&self, serial: &str, id: u8, payload: &[u8]) -> Vec<Alarm> {
        use dspi_proto::ext::timebase as tb;
        let mut s = self.lock();
        let announces = s.observed.get(serial).is_some_and(|o| o.time_base_events);
        let features = announces.then_some(tb::NOTIFY_FEAT_TIME_BASE);
        let Some(e) = tb::time_base_event(features, id, payload) else {
            return Vec::new();
        };
        let Some(u) = s.model.units.iter().find(|u| u.serial == serial) else {
            return Vec::new();
        };
        let transport = match e.time_base {
            tb::TB_ALINK_LINE => "alink",
            tb::TB_USB_SAMPLE => "usb",
            tb::TB_SPDIF => "spdif",
            _ => return Vec::new(),
        };
        let (unit, follower) = (u.id.clone(), u.spdif_follower());
        let name = if u.name.is_empty() {
            u.id.clone()
        } else {
            u.name.clone()
        };
        let action = s
            .observed
            .get(serial)
            .and_then(|o| o.spdif.as_ref())
            .map_or_else(
                || u.on_lock_error.as_str().to_owned(),
                |o| o.on_lock_error.clone(),
            );
        let expected = s
            .reanchoring
            .get(serial)
            .is_some_and(|until| Instant::now() <= *until);
        let reason = match e.reason.as_str() {
            // The hub's own write re-anchored it: no alarm.
            "reanchor" if expected => "none",
            r => r,
        };
        let lock_error = Some((
            align::spdif_lock_error_message(&name, &action, false),
            action != "play_unaligned",
        ));
        s.alarms.time_base_event(
            &unit,
            transport,
            e.state.as_str(),
            reason,
            follower,
            lock_error,
            now(),
        )
    }

    // -- WP AUSB2 --
    /// An 0xE3 from USB-direct unit `serial` of the model that announces it:
    /// `LOCK_ERROR` raises its lock-error alarm, `LOCK_OK` clears it. The
    /// changes.
    fn on_usb_event(&self, serial: &str, id: u8, payload: &[u8]) -> Vec<Alarm> {
        let mut s = self.lock();
        let announces = s.usb_events.get(serial).copied().unwrap_or(false);
        let features = announces.then_some(usb::NOTIFY_FEAT_ALINK_USB);
        let Some(e) = usb::alink_usb_event(features, id, payload) else {
            return Vec::new();
        };
        let Some(u) = s
            .model
            .units
            .iter()
            .find(|u| u.usb_direct() && u.serial == serial)
        else {
            return Vec::new();
        };
        let (unit, name) = (
            u.id.clone(),
            if u.name.is_empty() {
                u.id.clone()
            } else {
                u.name.clone()
            },
        );
        // What it does: as last read, else as the model asks.
        let action = s
            .observed
            .get(serial)
            .and_then(|o| o.usb.as_ref())
            .map_or_else(
                || u.on_lock_error.as_str().to_owned(),
                |o| o.on_lock_error.clone(),
            );
        let error = match e.reason {
            UsbEventReason::LockError => {
                let cause = align::lock_error_cause(e.state.as_str(), e.phase_state);
                Some((
                    align::lock_error_message(&name, &action, Some(cause)),
                    action != "play_unaligned",
                ))
            }
            UsbEventReason::LockOk => None,
            _ => return Vec::new(),
        };
        s.alarms.usb_lock_error(&unit, error, now())
    }

    /// A packet from the device with this serial (the relay's tap, fixup sw2x
    /// F5: by serial, so a unit that came back on another handle is still
    /// followed and a reused handle is never mistaken for it). An 0xE1 from a
    /// unit or a fan-out of the model that announces it raises or clears its
    /// alarm, which is broadcast on `relay`.
    pub(crate) fn on_packet(&self, relay: &Relay, serial: &str, packet: &[u8]) {
        let (Some(&id), Some(payload)) = (packet.get(1), packet.get(4..)) else {
            return;
        };
        if !Self::wants(packet) {
            return;
        }
        // -- WP ATHUBM --
        if id == dspi_proto::ext::timebase::NOTIFY_EVT_TIME_BASE {
            for a in &self.on_time_base_event(serial, id, payload) {
                relay.broadcast_event(&alarm_event(a));
            }
            return;
        }
        // -- WP AUSB2 --
        if id == usb::NOTIFY_EVT_ALINK_USB {
            for a in &self.on_usb_event(serial, id, payload) {
                relay.broadcast_event(&alarm_event(a));
            }
            return;
        }
        let changes = {
            let mut s = self.lock();
            let events = s.events.get(serial).copied().unwrap_or(false);
            let features = events.then_some(dspi_proto::ext::alink::NOTIFY_FEAT_ALINK_STATE);
            let Some(AlinkEvent::State(e)) = alink_event(features, id, payload) else {
                return;
            };
            let Some(unit) = element_of(&s.model, serial) else {
                return;
            };
            let serial = serial.to_owned();
            let expected = s
                .reanchoring
                .get(&serial)
                .is_some_and(|until| Instant::now() <= *until);
            let reason = match e.reason.as_str() {
                // The hub's own phase write: no alarm.
                "reanchor" if expected => "none",
                r => r,
            };
            s.alarms
                .event(&unit, e.state.as_str(), reason, e.reason.is_loss(), now())
        };
        for a in &changes {
            relay.broadcast_event(&alarm_event(a));
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_sw2 {
    use super::*;

    #[derive(Default)]
    struct Memory(Mutex<Option<SystemModel>>);

    impl SystemStore for Memory {
        fn load(&self) -> Option<SystemModel> {
            guard(&self.0).clone()
        }
        fn save(&self, model: &SystemModel) -> Result<(), HubError> {
            *guard(&self.0) = Some(model.clone());
            Ok(())
        }
    }

    #[test]
    fn wp_sw2_a_model_is_checked_kept_and_reloaded() {
        let store = Arc::new(Memory::default());
        let s = SystemShared::new(Some(store.clone()));
        assert_eq!(s.model().1, 0);
        let mut bad = align::tests_wp_sw2::cinema();
        bad.units[0].upstream = "nowhere".into();
        assert!(matches!(s.set_model(bad), Err(ModelRefused::Invalid(_))));
        assert!(guard(&store.0).is_none(), "nothing kept");
        let good = align::tests_wp_sw2::cinema();
        assert_eq!(s.set_model(good.clone()).unwrap(), 1);
        let again = SystemShared::new(Some(store));
        assert_eq!(again.model().0, good);
    }

    #[test]
    fn wp_sw2_a_loss_event_raises_an_alarm_on_the_relay() {
        let s = SystemShared::new(None);
        s.set_model(align::tests_wp_sw2::cinema()).unwrap();
        s.learned_events("S-B", true, false);
        s.learned_events("S-C", false, false);
        s.learned_events("S-fan", true, false);
        let relay = Relay::default();
        // 0xE1: holdover, class 1, LOS.
        let lost = [2, 0xE1, 0, 0, 3, 1, 2, 0, 1, 0, 0, 0];
        s.on_packet(&relay, "S-B", &lost);
        let a = s.alarms();
        assert_eq!(
            (a.len(), a[0].code.as_str(), a[0].subject.as_str()),
            (1, "alink_lost", "B")
        );
        // A device that does not announce the event, or not in the model: nothing.
        s.on_packet(&relay, "S-C", &lost);
        s.on_packet(&relay, "S-elsewhere", &lost);
        assert_eq!(s.alarms().len(), 1);
        let back = [2, 0xE1, 0, 1, 2, 1, 1, 0, 2, 0, 0, 0];
        s.on_packet(&relay, "S-B", &back);
        assert_eq!(s.lock().alarms.active(), 0);
        // -- fixup sw2x (F5) -- a fan-out's loss alarms under its own id.
        s.on_packet(&relay, "S-fan", &lost);
        assert_eq!(s.alarms()[0].subject, "fan");
        assert!(s.watches("S-fan") && s.watches("S-B") && !s.watches("S-tx"));
    }
}
