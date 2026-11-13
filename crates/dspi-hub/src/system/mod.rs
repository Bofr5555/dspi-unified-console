//! The hub's system model for the audio link (WP SW2, SYS2 section 7): the
//! installation ([`model`]), the alignment engine ([`align`]), scenes applied at
//! a frame ([`scene`]) and link alarms ([`alarms`]), kept in the hub's store and
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
pub mod model;
mod run;
pub mod scene;

pub use run::{AlignReport, SceneReport, SystemStatus, WriteResult};

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use dspi_proto::ext::audio_link::{LinkEvent, link_event};
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
}

/// Where the model persists: the hub's store (`hub-store.json`, `system`).
pub trait SystemStore: Send + Sync {
    /// The model kept from last time, if one was kept and is valid.
    fn load(&self) -> Option<SystemModel>;
    /// Replace the model kept.
    fn save(&self, model: &SystemModel) -> Result<(), HubError>;
}

#[derive(Default)]
struct State {
    model: SystemModel,
    /// Bumped by every accepted `system.set`.
    revision: u64,
    alarms: AlarmBook,
    /// The last reads, by serial.
    observed: BTreeMap<String, Observed>,
    /// Which handle is which serial, at the last read, for the event tap; with
    /// the device's `EXT_NOTIFY_FEATURES` bit 2.
    handles: BTreeMap<u8, (String, bool)>,
    alignment: Option<Alignment>,
    /// Units the hub has just re-anchored itself (a new `EXT_LINK_PHASE`), by
    /// serial: their `REANCHOR` event before this is no alarm.
    reanchoring: BTreeMap<String, Instant>,
}

/// The system's state, shared by the hub's handles and the relay's tap.
pub(crate) struct SystemShared {
    state: Mutex<State>,
    store: Option<Arc<dyn SystemStore>>,
    /// Keeps saves in the order of the changes. Above `state`.
    saving: Mutex<()>,
}

impl std::fmt::Debug for SystemShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SystemShared")
    }
}

/// How long after the hub writes a unit's phase its re-anchor is expected.
const REANCHOR_EXPECTED: Duration = Duration::from_secs(2);

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

impl SystemShared {
    /// The model from `store`, or an empty one.
    pub(crate) fn new(store: Option<Arc<dyn SystemStore>>) -> Self {
        let model = store.as_ref().and_then(|s| s.load()).unwrap_or_default();
        Self {
            state: Mutex::new(State {
                model,
                ..State::default()
            }),
            store,
            saving: Mutex::new(()),
        }
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
    pub(crate) fn set_model(&self, model: SystemModel) -> Result<u64, ModelRefused> {
        model.validate().map_err(ModelRefused::Invalid)?;
        let _in_order = guard(&self.saving);
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

    /// Keep what a read found; the alarm changes to broadcast.
    pub(crate) fn observed(
        &self,
        observed: BTreeMap<String, Observed>,
        handles: BTreeMap<u8, (String, bool)>,
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
        s.observed = observed;
        s.handles = handles;
        s.alignment = Some(alignment);
        changes
    }

    /// The hub is about to write unit `serial`'s phase: its re-anchor follows.
    pub(crate) fn expect_reanchor(&self, serial: &str) {
        if let Some(until) = Instant::now().checked_add(REANCHOR_EXPECTED) {
            self.lock().reanchoring.insert(serial.to_owned(), until);
        }
    }

    /// A packet from the device at `handle` (the relay's tap): an 0xE1 from a
    /// unit of the model that announces it raises or clears its alarm, which is
    /// broadcast on `relay`. Cheap for every other packet.
    pub(crate) fn on_packet(&self, relay: &Relay, handle: u8, packet: &[u8]) {
        let (Some(&id), Some(payload)) = (packet.get(1), packet.get(4..)) else {
            return;
        };
        if id != dspi_proto::ext::audio_link::NOTIFY_EVT_LINK_STATE {
            return;
        }
        let changes = {
            let mut s = self.lock();
            let Some((serial, events)) = s.handles.get(&handle).cloned() else {
                return;
            };
            let features = events.then_some(dspi_proto::ext::audio_link::NOTIFY_FEAT_LINK_STATE);
            let Some(LinkEvent::State(e)) = link_event(features, id, payload) else {
                return;
            };
            let Some(unit) = s.model.units.iter().find(|u| u.serial == serial) else {
                return;
            };
            let unit = unit.id.clone();
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
        s.lock().handles.insert(3, ("S-B".into(), true));
        s.lock().handles.insert(4, ("S-C".into(), false));
        let relay = Relay::default();
        // 0xE1: holdover, class 1, LOS.
        let lost = [2, 0xE1, 0, 0, 3, 1, 2, 0, 1, 0, 0, 0];
        s.on_packet(&relay, 3, &lost);
        let a = s.alarms();
        assert_eq!(
            (a.len(), a[0].code.as_str(), a[0].subject.as_str()),
            (1, "link_lost", "B")
        );
        // A device that does not announce the event, or an unknown handle: nothing.
        s.on_packet(&relay, 4, &lost);
        s.on_packet(&relay, 9, &lost);
        assert_eq!(s.alarms().len(), 1);
        let back = [2, 0xE1, 0, 1, 2, 1, 1, 0, 2, 0, 0, 0];
        s.on_packet(&relay, 3, &back);
        assert_eq!(s.lock().alarms.active(), 0);
    }
}
