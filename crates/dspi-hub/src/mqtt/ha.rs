//! Home Assistant over the hub's MQTT connection (WP P2HA, PLAN-P2 I6, OD-P2c/P2e;
//! `docs/integrator/hub-api.md` section 13.2).
//!
//! `ha: {enabled, key}` in `mqtt.json` (P2MQ's setting) turns it on. While it is on and
//! MQTT is connected, every device the hub sees gets retained discovery configs under
//! `homeassistant/<component>/<hub>_<serial>_<entity>/config`, its state on retained
//! `<base>/<serial>/<entity>/state` topics, and `<base>/<serial>/availability`
//! (`online`/`offline`, beside the hub's own `<base>/status`):
//!
//! | Entity | Component | State from | Set as |
//! |---|---|---|---|
//! | `master_volume` | `number` (the registry's range) | `PARAM_CHANGED` of the master volume | `param.set vol.master` |
//! | `mute` | `switch` | `PARAM_CHANGED` of the user mute | `param.set vol.mute` |
//! | `preset` | `select` (the device's names) | `PRESET_LOADED` | `preset.load {slot}` |
//! | `identify` | `button` | | `device.identify` (10 s, no tone) |
//! | `temperature`, `cpu` | `sensor` | `Health` | |
//! | `clip` | `binary_sensor` | `Health`'s clip latch | |
//!
//! A device the hub forgets (its grace ended, `DeviceRemoved`) has its configs, states
//! and availability cleared (empty retained payloads), so Home Assistant removes it.
//!
//! **Who writes (OD-P2c, PLAN-P2 M2).** A `set` acts as the API key in `ha.key`: a
//! principal session opened with [`HubHandle::open_principal`] (a remote-range id, the
//! key's `cid`, role and name), and the message goes through
//! [`crate::server::control_plane`], the same chain a WebSocket or REST request takes:
//! the lock's, the guarded apply's and the limits' handlers, then the parameter API's
//! executor (P2PA). So the role, the limits, the installer lock, the device lock, a
//! pending guarded apply and the read-only rule judge it, and the write is attributed
//! to the key's name. Nothing here sends a raw transfer, and nothing writes above the
//! key's role. A key that is revoked or not an API key writes nothing.
//!
//! **Hostile input.** Whatever arrives on a `set` topic is network data: a payload
//! over [`MAX_PAYLOAD`] bytes, a retained message (a write is never replayed), a
//! topic that is not `<base>/<serial>/<entity>/set` of a device the hub sees, text that
//! is not the entity's (`ON`/`OFF`, a finite number in the registry's range, a known
//! preset label, `PRESS`) is dropped and counted, never a panic. The client's thread
//! only puts a message on a bounded queue ([`INBOUND_CAP`], the oldest go); the MQTT
//! pump parses it; the writes run on this module's own worker thread (`hub-ha`), from
//! a bounded job queue ([`JOB_CAP`]) where a newer set of the same entity replaces an
//! older one still waiting.
//!
//! **Threads.** [`HaRuntime`] lives in P2MQ's pump (`service.rs`), which owns the
//! client: it holds the pure [`HaState`] and the publishing [`Outbox`] (latest value per
//! topic, flushed while the client's queue has room). Its [`Worker`] makes the calls
//! that block on a device (writes, and the reads that fill the state: volume, mute and
//! the preset names) and hands back [`Feedback`]. A refused set publishes the state
//! again as it was, so Home Assistant's control moves back.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use dspi_proto::generated as g;
use dspi_proto::registry::{self, Kind};
use dspi_session::notify::{self, Event};
use serde_json::{Value, json};

use super::client::{MqttClient, MqttEvent};
use super::codec::QoS;
use super::service::HaSetting;
use crate::auth::AuthError;
use crate::bus::{Bus, BusMessage, Subscription};
use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::session::{Role, Session};
use crate::store::ClientKind;

/// Home Assistant's discovery prefix.
pub const DISCOVERY_PREFIX: &str = "homeassistant";
/// The name of the key minted when `ha` is turned on without one.
pub const KEY_NAME: &str = "Home Assistant";
/// How long the Identify button blinks.
pub const IDENTIFY_SECONDS: u64 = 10;
/// The longest `set` payload taken, in bytes.
pub const MAX_PAYLOAD: usize = 64;
/// The longest `set` topic taken, in bytes.
pub const MAX_TOPIC: usize = 512;
/// `set` messages waiting for the pump; the oldest go past it.
pub const INBOUND_CAP: usize = 64;
/// Jobs waiting for the worker; the oldest go past it.
pub const JOB_CAP: usize = 64;
/// Devices with entities at once (a handle is a byte).
pub const MAX_DEVICES: usize = 256;
/// Topics waiting to be published; the oldest go past it.
pub const MAX_OUTBOX: usize = 4096;
/// Feedback waiting for the pump; the oldest go past it.
const FEEDBACK_CAP: usize = 256;
/// The HA bus subscription's queue.
const BUS_CAPACITY: usize = 1024;
/// Bus messages read per pump pass, at most.
const BUS_BATCH: usize = 256;
/// The outbox is flushed while the client holds fewer publishes than this.
const FLUSH_BELOW: usize = 128;
/// Publishes handed to the client per pump pass, at most.
const FLUSH_BATCH: usize = 64;
/// Preset slots on every DSPi (`MAX_PRESETS`).
const PRESET_SLOTS: usize = 10;
/// The longest preset label.
const MAX_LABEL_CHARS: usize = 64;
/// How long the idle worker waits before it looks at its stop flag again.
const WORKER_WAIT: Duration = Duration::from_millis(500);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Entities and topics ─────────────────────────────────────────────────────

/// One entity of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entity {
    /// The master volume, a `number`.
    MasterVolume,
    /// The user mute, a `switch`.
    Mute,
    /// The preset, a `select`.
    Preset,
    /// Identify, a `button`.
    Identify,
    /// Temperature, a `sensor`.
    Temperature,
    /// CPU load, a `sensor`.
    Cpu,
    /// The clip latch, a `binary_sensor`.
    Clip,
}

impl Entity {
    /// Every entity, in the order they are published.
    pub const ALL: [Entity; 7] = [
        Entity::MasterVolume,
        Entity::Mute,
        Entity::Preset,
        Entity::Identify,
        Entity::Temperature,
        Entity::Cpu,
        Entity::Clip,
    ];

    /// Its topic level and the end of its unique id.
    pub fn word(self) -> &'static str {
        match self {
            Entity::MasterVolume => "master_volume",
            Entity::Mute => "mute",
            Entity::Preset => "preset",
            Entity::Identify => "identify",
            Entity::Temperature => "temperature",
            Entity::Cpu => "cpu",
            Entity::Clip => "clip",
        }
    }

    /// Its Home Assistant component.
    pub fn component(self) -> &'static str {
        match self {
            Entity::MasterVolume => "number",
            Entity::Mute => "switch",
            Entity::Preset => "select",
            Entity::Identify => "button",
            Entity::Temperature | Entity::Cpu => "sensor",
            Entity::Clip => "binary_sensor",
        }
    }

    /// The entity of a topic level.
    pub fn from_word(word: &str) -> Option<Entity> {
        Entity::ALL.into_iter().find(|e| e.word() == word)
    }

    /// Whether it has a `set` topic.
    pub fn settable(self) -> bool {
        matches!(
            self,
            Entity::MasterVolume | Entity::Mute | Entity::Preset | Entity::Identify
        )
    }

    fn has_state(self) -> bool {
        self != Entity::Identify
    }

    fn label(self) -> &'static str {
        match self {
            Entity::MasterVolume => "Master volume",
            Entity::Mute => "Mute",
            Entity::Preset => "Preset",
            Entity::Identify => "Identify",
            Entity::Temperature => "Temperature",
            Entity::Cpu => "CPU load",
            Entity::Clip => "Clip",
        }
    }
}

/// A serial as a topic level: `None` for one that is none (empty, `/`, a wildcard, a
/// control character, longer than 64 bytes), as P2MQ's event topics.
pub fn serial_level(serial: &str) -> Option<&str> {
    let fine = !serial.is_empty()
        && serial.len() <= 64
        && !serial
            .chars()
            .any(|c| matches!(c, '/' | '+' | '#') || c.is_control());
    fine.then_some(serial)
}

/// Letters, digits, `-` and `_` only: a part of a unique id.
fn id_part(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// `<hub>_<serial>_<entity>`, letters, digits, `-` and `_` only.
pub fn unique_id(hub_id: &str, serial: &str, entity: Entity) -> String {
    format!("{}_{}_{}", id_part(hub_id), id_part(serial), entity.word())
}

/// `homeassistant/<component>/<hub>_<serial>_<entity>/config`.
pub fn config_topic(hub_id: &str, serial: &str, entity: Entity) -> String {
    format!(
        "{DISCOVERY_PREFIX}/{}/{}/config",
        entity.component(),
        unique_id(hub_id, serial, entity)
    )
}

/// `<base>/<serial>/<entity>/state`.
pub fn state_topic(base: &str, serial: &str, entity: Entity) -> String {
    format!("{base}/{serial}/{}/state", entity.word())
}

/// `<base>/<serial>/<entity>/set`.
pub fn set_topic(base: &str, serial: &str, entity: Entity) -> String {
    format!("{base}/{serial}/{}/set", entity.word())
}

/// `<base>/<serial>/availability`.
pub fn availability_topic(base: &str, serial: &str) -> String {
    format!("{base}/{serial}/availability")
}

/// What the hub subscribes to: `<base>/+/+/set`.
pub fn set_filter(base: &str) -> String {
    format!("{base}/+/+/set")
}

/// The master volume's range, from the parameter registry.
pub fn volume_range() -> (f64, f64) {
    match registry::by_path("vol.master").map(|d| d.kind) {
        Some(Kind::Float { min, max, .. }) => (f64::from(min), f64::from(max)),
        _ => (-128.0, 0.0),
    }
}

// ── Inbound: what a `set` topic says ────────────────────────────────────────

/// A `set` that was understood.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// The master volume, dB, inside the registry's range.
    Volume(f64),
    /// Mute on or off.
    Mute(bool),
    /// The preset with this label.
    Preset(String),
    /// Press Identify.
    Identify,
}

/// Why a `set` was dropped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InboundError {
    /// Not `<base>/<serial>/<entity>/set` of a settable entity.
    #[error("not a set topic of the hub")]
    NotOurs,
    /// Over [`MAX_PAYLOAD`] bytes.
    #[error("the payload is too long")]
    TooLong,
    /// Not UTF-8.
    #[error("the payload is not text")]
    NotText,
    /// Not a value of the entity.
    #[error("not a value of the entity: {0}")]
    BadValue(&'static str),
}

/// Read a message on a `set` topic. Any bytes in, a typed answer out, never a panic.
pub fn parse_set(
    base: &str,
    topic: &str,
    payload: &[u8],
) -> Result<(String, Entity, Command), InboundError> {
    let rest = topic
        .strip_prefix(base)
        .and_then(|r| r.strip_prefix('/'))
        .ok_or(InboundError::NotOurs)?;
    let mut parts = rest.split('/');
    let (Some(serial), Some(word), Some("set"), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(InboundError::NotOurs);
    };
    let serial = serial_level(serial).ok_or(InboundError::NotOurs)?;
    let entity = Entity::from_word(word)
        .filter(|e| e.settable())
        .ok_or(InboundError::NotOurs)?;
    if payload.len() > MAX_PAYLOAD {
        return Err(InboundError::TooLong);
    }
    let text = std::str::from_utf8(payload)
        .map_err(|_| InboundError::NotText)?
        .trim();
    let command = match entity {
        Entity::MasterVolume => {
            let v: f64 = text
                .parse()
                .map_err(|_| InboundError::BadValue("a number of dB"))?;
            let (min, max) = volume_range();
            if !v.is_finite() || v < min || v > max {
                return Err(InboundError::BadValue("outside the master volume's range"));
            }
            Command::Volume(v)
        }
        Entity::Mute => {
            if text.eq_ignore_ascii_case("ON") {
                Command::Mute(true)
            } else if text.eq_ignore_ascii_case("OFF") {
                Command::Mute(false)
            } else {
                return Err(InboundError::BadValue("ON or OFF"));
            }
        }
        Entity::Preset => {
            if text.is_empty() || text.chars().any(char::is_control) {
                return Err(InboundError::BadValue("a preset's name"));
            }
            Command::Preset(text.to_owned())
        }
        // Identify, the last settable one.
        _ => {
            if !text.eq_ignore_ascii_case("PRESS") {
                return Err(InboundError::BadValue("PRESS"));
            }
            Command::Identify
        }
    };
    Ok((serial.to_owned(), entity, command))
}

// ── The publishing outbox ───────────────────────────────────────────────────

/// What waits to be published: the latest payload per topic, in the order the topics
/// first came. Every HA publish is QoS 1 and retained (an empty payload removes).
#[derive(Debug, Default)]
pub struct Outbox {
    order: VecDeque<String>,
    items: HashMap<String, Vec<u8>>,
    dropped: u64,
}

impl Outbox {
    /// Queue `payload` for `topic`, replacing one still waiting.
    pub fn put(&mut self, topic: String, payload: Vec<u8>) {
        if let Some(waiting) = self.items.get_mut(&topic) {
            *waiting = payload;
            return;
        }
        while self.order.len() >= MAX_OUTBOX {
            if let Some(old) = self.order.pop_front() {
                self.items.remove(&old);
                self.dropped = self.dropped.saturating_add(1);
            }
        }
        self.order.push_back(topic.clone());
        self.items.insert(topic, payload);
    }

    /// The oldest waiting topic and its payload.
    pub fn pop(&mut self) -> Option<(String, Vec<u8>)> {
        while let Some(topic) = self.order.pop_front() {
            if let Some(payload) = self.items.remove(&topic) {
                return Some((topic, payload));
            }
        }
        None
    }

    /// Topics waiting.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Nothing waits.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Topics dropped past [`MAX_OUTBOX`].
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Everything waiting, oldest first (tests).
    pub fn drain(&mut self) -> Vec<(String, Vec<u8>)> {
        std::iter::from_fn(|| self.pop()).collect()
    }
}

// ── The state ───────────────────────────────────────────────────────────────

/// What the worker is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// List the hub's devices and refresh each online one.
    Scan,
    /// Read a device's name, volume, mute and presets.
    Refresh(String),
    /// Send this control-plane message (`param.set`, `preset.load`, `device.identify`)
    /// as the key's principal; `entity` is what to publish again if it is refused.
    Write {
        /// The device.
        serial: String,
        /// The entity it sets.
        entity: Entity,
        /// The message, without its `id`.
        msg: Value,
    },
}

/// What the worker found.
#[derive(Debug, Clone, PartialEq)]
pub enum Feedback {
    /// A device as the registry has it.
    Device {
        /// Its serial.
        serial: String,
        /// The name clients show.
        name: String,
        /// Its firmware.
        fw: String,
        /// Its platform's name.
        platform: String,
        /// Attached now.
        online: bool,
    },
    /// Values read or written.
    Values {
        /// The device.
        serial: String,
        /// The master volume, dB.
        volume: Option<f64>,
        /// The user mute.
        mute: Option<bool>,
        /// The used preset slots and their labels.
        presets: Option<Vec<(u8, String)>>,
        /// The loaded preset, if one was read (`Some(None)`: none is).
        active: Option<Option<u8>>,
    },
    /// A set was refused: publish the entity's state again as it is.
    Refused {
        /// The device.
        serial: String,
        /// The entity.
        entity: Entity,
    },
}

/// One device's entities.
#[derive(Debug, Clone, Default, PartialEq)]
struct Device {
    name: String,
    fw: String,
    platform: String,
    /// From `Facts` (board and hardware revision), over `platform`.
    model: Option<String>,
    online: bool,
    volume: Option<f64>,
    mute: Option<bool>,
    presets: Vec<(u8, String)>,
    active: Option<u8>,
    temp: Option<f32>,
    cpu: Option<u8>,
    clip: Option<bool>,
}

fn on_off(b: bool) -> &'static str {
    if b { "ON" } else { "OFF" }
}

/// The used slots' labels: the name, or `Slot <n>` for an unnamed one; a name used
/// twice gets ` (slot <n>)` the second time. Labels are unique and at most 64 chars.
pub fn preset_labels(slots: &[(u8, String, bool)]) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, String)> = Vec::new();
    for (slot, name, used) in slots {
        if !*used {
            continue;
        }
        let name: String = name
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_LABEL_CHARS)
            .collect();
        let mut label = if name.is_empty() {
            format!("Slot {slot}")
        } else {
            name
        };
        if out.iter().any(|(_, l)| *l == label) {
            let suffix = format!(" (slot {slot})");
            let keep = MAX_LABEL_CHARS.saturating_sub(suffix.chars().count());
            label = label.chars().take(keep).collect::<String>() + &suffix;
        }
        out.push((*slot, label));
    }
    out
}

/// The bytes of `[at, at + len)` inside a change of `bytes` at `offset`.
fn covered(offset: u16, bytes: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    let rel = at.checked_sub(usize::from(offset))?;
    bytes.get(rel..rel.checked_add(len)?)
}

/// Home Assistant's view of the hub's devices: pure, fed the bus and the worker,
/// writing to an [`Outbox`]. Publishes a topic only when its payload changed.
#[derive(Debug)]
pub struct HaState {
    base: String,
    hub: String,
    devices: BTreeMap<String, Device>,
    /// The last payload put out per topic.
    published: HashMap<String, Vec<u8>>,
}

impl HaState {
    /// No devices yet. `base` is P2MQ's base topic, `hub` the hub's id.
    pub fn new(base: &str, hub: &str) -> Self {
        Self {
            base: base.to_owned(),
            hub: hub.to_owned(),
            devices: BTreeMap::new(),
            published: HashMap::new(),
        }
    }

    /// The serials with entities.
    pub fn serials(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }

    fn emit(&mut self, out: &mut Outbox, topic: String, payload: Vec<u8>, force: bool) {
        if !force && self.published.get(&topic) == Some(&payload) {
            return;
        }
        if payload.is_empty() {
            self.published.remove(&topic);
        } else {
            self.published.insert(topic.clone(), payload.clone());
        }
        out.put(topic, payload);
    }

    /// The device, made if it is new (and there is room for it).
    fn device(&mut self, serial: &str) -> Option<&mut Device> {
        serial_level(serial)?;
        if !self.devices.contains_key(serial) {
            if self.devices.len() >= MAX_DEVICES {
                log::warn!("hub: Home Assistant has {MAX_DEVICES} devices; {serial} left out");
                return None;
            }
            self.devices.insert(serial.to_owned(), Device::default());
        }
        self.devices.get_mut(serial)
    }

    fn device_block(&self, serial: &str, d: &Device) -> Value {
        let name = if d.name.trim().is_empty() {
            format!("DSPi {serial}")
        } else {
            d.name.clone()
        };
        let model = d.model.clone().unwrap_or_else(|| d.platform.clone());
        let mut block = json!({
            "identifiers": [format!("dspi_{}", id_part(serial))],
            "name": name,
            "serial_number": serial,
            "manufacturer": "DSPi",
        });
        if let Some(map) = block.as_object_mut() {
            if !d.fw.is_empty() {
                map.insert("sw_version".into(), d.fw.clone().into());
            }
            if !model.is_empty() {
                map.insert("model".into(), model.into());
            }
        }
        block
    }

    /// An entity's discovery config; `None` for one it cannot have now (a preset
    /// select with no used slot).
    fn config(&self, serial: &str, d: &Device, e: Entity) -> Option<Value> {
        let base = &self.base;
        let mut c = json!({
            "name": e.label(),
            "unique_id": unique_id(&self.hub, serial, e),
            "object_id": unique_id(&self.hub, serial, e),
            "availability": [
                {"topic": super::service::status_topic(base)},
                {"topic": availability_topic(base, serial)},
            ],
            "availability_mode": "all",
            "device": self.device_block(serial, d),
            "origin": {"name": "DSPi hub"},
        });
        let map = c.as_object_mut()?;
        if e.has_state() {
            map.insert("state_topic".into(), state_topic(base, serial, e).into());
        }
        if e.settable() {
            map.insert("command_topic".into(), set_topic(base, serial, e).into());
        }
        match e {
            Entity::MasterVolume => {
                let (min, max) = volume_range();
                map.insert("min".into(), min.into());
                map.insert("max".into(), max.into());
                map.insert("step".into(), 0.5.into());
                map.insert("mode".into(), "slider".into());
                map.insert("unit_of_measurement".into(), "dB".into());
            }
            Entity::Mute => {
                map.insert("icon".into(), "mdi:volume-off".into());
            }
            Entity::Preset => {
                if d.presets.is_empty() {
                    return None;
                }
                let options: Vec<&str> = d.presets.iter().map(|(_, l)| l.as_str()).collect();
                map.insert("options".into(), json!(options));
            }
            Entity::Identify => {
                map.insert("device_class".into(), "identify".into());
                map.insert("payload_press".into(), "PRESS".into());
            }
            Entity::Temperature => {
                map.insert("device_class".into(), "temperature".into());
                map.insert("unit_of_measurement".into(), "°C".into());
                map.insert("state_class".into(), "measurement".into());
            }
            Entity::Cpu => {
                map.insert("unit_of_measurement".into(), "%".into());
                map.insert("state_class".into(), "measurement".into());
                map.insert("icon".into(), "mdi:chip".into());
            }
            Entity::Clip => {
                map.insert("device_class".into(), "problem".into());
            }
        }
        Some(c)
    }

    /// An entity's state text, if known.
    fn state_of(d: &Device, e: Entity) -> Option<String> {
        match e {
            Entity::MasterVolume => d.volume.map(|v| format!("{v}")),
            Entity::Mute => d.mute.map(|m| on_off(m).to_owned()),
            Entity::Preset => {
                let slot = d.active?;
                d.presets
                    .iter()
                    .find(|(s, _)| *s == slot)
                    .map(|(_, l)| l.clone())
            }
            Entity::Identify => None,
            Entity::Temperature => d.temp.map(|t| format!("{t:.1}")),
            Entity::Cpu => d.cpu.map(|c| c.to_string()),
            Entity::Clip => d.clip.map(|c| on_off(c).to_owned()),
        }
    }

    fn publish_state(&mut self, out: &mut Outbox, serial: &str, e: Entity, force: bool) {
        let Some(text) = self.devices.get(serial).and_then(|d| Self::state_of(d, e)) else {
            return;
        };
        let topic = state_topic(&self.base, serial, e);
        self.emit(out, topic, text.into_bytes(), force);
    }

    /// Availability, every config and every known state of `serial`.
    fn publish_device(&mut self, out: &mut Outbox, serial: &str, force: bool) {
        let Some(d) = self.devices.get(serial).cloned() else {
            return;
        };
        let avail = availability_topic(&self.base, serial);
        let word: &[u8] = if d.online { b"online" } else { b"offline" };
        self.emit(out, avail, word.to_vec(), force);
        for e in Entity::ALL {
            let topic = config_topic(&self.hub, serial, e);
            match self.config(serial, &d, e) {
                Some(c) => self.emit(out, topic, c.to_string().into_bytes(), force),
                // Removed if it was there.
                None if self.published.contains_key(&topic) => {
                    self.emit(out, topic, Vec::new(), true);
                }
                None => {}
            }
            self.publish_state(out, serial, e, force);
        }
    }

    /// Everything again (after a reconnect: the broker may have lost it).
    pub fn republish_all(&mut self, out: &mut Outbox) {
        for serial in self.serials() {
            self.publish_device(out, &serial, true);
        }
    }

    /// Remove `serial`'s entities from Home Assistant: empty retained configs, states
    /// and availability.
    pub fn remove_device(&mut self, out: &mut Outbox, serial: &str) {
        if self.devices.remove(serial).is_none() {
            return;
        }
        for e in Entity::ALL {
            let topic = config_topic(&self.hub, serial, e);
            self.emit(out, topic, Vec::new(), true);
            if e.has_state() {
                let topic = state_topic(&self.base, serial, e);
                self.emit(out, topic, Vec::new(), true);
            }
        }
        let topic = availability_topic(&self.base, serial);
        self.emit(out, topic, Vec::new(), true);
    }

    /// Remove every device (Home Assistant turned off).
    pub fn remove_all(&mut self, out: &mut Outbox) {
        for serial in self.serials() {
            self.remove_device(out, &serial);
        }
    }

    /// What a bus message changes; the jobs it asks of the worker.
    pub fn on_bus(&mut self, out: &mut Outbox, m: &BusMessage) -> Vec<Job> {
        match m {
            BusMessage::DeviceOnline { serial, .. } => {
                let Some(d) = self.device(serial) else {
                    return Vec::new();
                };
                d.online = true;
                self.publish_device(out, serial, false);
                vec![Job::Refresh(serial.clone())]
            }
            BusMessage::DeviceOffline { serial, .. } => {
                if let Some(d) = self.devices.get_mut(serial) {
                    d.online = false;
                    self.publish_device(out, serial, false);
                }
                Vec::new()
            }
            BusMessage::DeviceRemoved { serial, .. } => {
                self.remove_device(out, serial);
                Vec::new()
            }
            BusMessage::Notification { serial, packet, .. } => {
                if !self.devices.contains_key(serial) {
                    return Vec::new();
                }
                self.on_notification(out, serial, packet)
            }
            BusMessage::Health {
                serial,
                cpu,
                clip_mask,
                temp_c,
                ..
            } => {
                let Some(d) = self.devices.get_mut(serial) else {
                    return Vec::new();
                };
                if let Some(c) = cpu {
                    d.cpu = Some(*c);
                }
                if let Some(t) = temp_c.filter(|t| t.is_finite()) {
                    d.temp = Some(t);
                }
                d.clip = Some(*clip_mask != 0);
                for e in [Entity::Temperature, Entity::Cpu, Entity::Clip] {
                    self.publish_state(out, serial, e, false);
                }
                Vec::new()
            }
            BusMessage::Facts {
                serial,
                identify,
                fw,
                ..
            } => {
                let Some(d) = self.devices.get_mut(serial) else {
                    return Vec::new();
                };
                if !fw.is_empty() {
                    d.fw.clone_from(fw);
                }
                if let Some(id) = identify {
                    let board = id.board.trim();
                    let rev = id.hw_rev.trim();
                    d.model = match (board.is_empty(), rev.is_empty()) {
                        (true, _) => d.model.take(),
                        (false, true) => Some(board.to_owned()),
                        (false, false) => Some(format!("{board} ({rev})")),
                    };
                }
                self.publish_device(out, serial, false);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_notification(&mut self, out: &mut Outbox, serial: &str, packet: &[u8]) -> Vec<Job> {
        let Ok((_, event)) = notify::decode(packet) else {
            return Vec::new();
        };
        let Some(d) = self.devices.get_mut(serial) else {
            return Vec::new();
        };
        match event {
            Event::MasterVolume(v) if v.is_finite() => {
                d.volume = Some(f64::from(v));
                self.publish_state(out, serial, Entity::MasterVolume, false);
            }
            Event::ParamChanged { offset, bytes, .. } => {
                let volume = covered(offset, &bytes, g::OFF_MASTER_VOLUME, 4)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map(f32::from_le_bytes)
                    .filter(|v| v.is_finite());
                let mute = g::OFF_USER_VOLUME
                    .checked_add(4)
                    .and_then(|at| covered(offset, &bytes, at, 1))
                    .and_then(|b| b.first().copied());
                if let Some(v) = volume {
                    d.volume = Some(f64::from(v));
                }
                if let Some(m) = mute {
                    d.mute = Some(m != 0);
                }
                if volume.is_some() {
                    self.publish_state(out, serial, Entity::MasterVolume, false);
                }
                if mute.is_some() {
                    self.publish_state(out, serial, Entity::Mute, false);
                }
            }
            Event::PresetLoaded { slot } => {
                d.active = Some(slot);
                self.publish_state(out, serial, Entity::Preset, false);
                // A preset may carry the volume and the mute: read them again.
                return vec![Job::Refresh(serial.to_owned())];
            }
            Event::BulkInvalidated { .. } => return vec![Job::Refresh(serial.to_owned())],
            _ => {}
        }
        Vec::new()
    }

    /// What the worker found.
    pub fn on_feedback(&mut self, out: &mut Outbox, f: Feedback) {
        match f {
            Feedback::Device {
                serial,
                name,
                fw,
                platform,
                online,
            } => {
                let Some(d) = self.device(&serial) else {
                    return;
                };
                d.name = name;
                if !fw.is_empty() {
                    d.fw = fw;
                }
                d.platform = platform;
                d.online = online;
                self.publish_device(out, &serial, false);
            }
            Feedback::Values {
                serial,
                volume,
                mute,
                presets,
                active,
            } => {
                let Some(d) = self.devices.get_mut(&serial) else {
                    return;
                };
                if let Some(v) = volume.filter(|v| v.is_finite()) {
                    d.volume = Some(v);
                }
                if let Some(m) = mute {
                    d.mute = Some(m);
                }
                let presets_changed = presets.is_some_and(|p| {
                    let changed = p != d.presets;
                    d.presets = p;
                    changed
                });
                if let Some(a) = active {
                    d.active = a;
                }
                if presets_changed {
                    self.publish_device(out, &serial, false);
                } else {
                    for e in [Entity::MasterVolume, Entity::Mute, Entity::Preset] {
                        self.publish_state(out, &serial, e, false);
                    }
                }
            }
            Feedback::Refused { serial, entity } => {
                self.publish_state(out, &serial, entity, true);
            }
        }
    }

    /// A `set` understood: the job that carries it out, or `None` (a device without
    /// entities, a preset label it has not). A preset label it has not puts the state
    /// back.
    pub fn command(
        &mut self,
        out: &mut Outbox,
        serial: &str,
        entity: Entity,
        command: Command,
    ) -> Option<Job> {
        let d = self.devices.get(serial)?;
        let msg = match command {
            Command::Volume(v) => json!({"t": "param.set", "serial": serial,
                "path": "vol.master", "idx": [], "value": v}),
            Command::Mute(m) => json!({"t": "param.set", "serial": serial,
                "path": "vol.mute", "idx": [], "value": m}),
            Command::Preset(label) => {
                let Some((slot, _)) = d.presets.iter().find(|(_, l)| *l == label) else {
                    self.publish_state(out, serial, Entity::Preset, true);
                    return None;
                };
                json!({"t": "preset.load", "serial": serial, "slot": slot})
            }
            Command::Identify => {
                json!({"t": "device.identify", "serial": serial, "seconds": IDENTIFY_SECONDS})
            }
        };
        Some(Job::Write {
            serial: serial.to_owned(),
            entity,
            msg,
        })
    }
}

// ── The key (`mqtt.set`) ────────────────────────────────────────────────────

/// `mqtt.set`'s `ha` against the hub's keys (OD-P2c/P2e): `ha` on without a key mints a
/// control key named [`KEY_NAME`] (its `cid` put in `next`; the key itself is never
/// shown, Home Assistant needs only its principal); a `key` that is new or newly turned
/// on must be an API key. `ha` off, or unchanged, is taken as it is. Answers the `cid`
/// minted, or the refusal's code and line.
pub fn settle_key(
    hub: &HubHandle,
    current: HaSetting,
    next: &mut HaSetting,
) -> Result<Option<u32>, (&'static str, String)> {
    if !next.enabled || (*next == current && next.key.is_some()) {
        return Ok(None);
    }
    let Some(auth) = hub.inner.services.auth() else {
        return Err((
            "unsupported",
            "Home Assistant acts as an API key, and this hub keeps none (no network store)".into(),
        ));
    };
    if let Some(cid) = next.key {
        return match auth.clients().into_iter().find(|c| c.cid == cid) {
            Some(c) if c.kind == ClientKind::Api => Ok(None),
            Some(_) => Err((
                "bad_request",
                format!("ha.key: client {cid} is a console, not an API key"),
            )),
            None => Err((
                "bad_request",
                format!("ha.key: there is no API key {cid}; null mints a new one"),
            )),
        };
    }
    match auth.mint_api_key(KEY_NAME, Role::Control, false) {
        Ok(minted) => {
            log::info!(
                "hub: API key {} ({KEY_NAME}, control) minted for Home Assistant",
                minted.cid
            );
            next.key = Some(minted.cid);
            Ok(Some(minted.cid))
        }
        Err(AuthError::TableFull) => Err((
            "conflict",
            "64 paired clients and API keys at most: revoke one, or give ha.key".into(),
        )),
        Err(_) => Err(("internal", "no key could be made or stored".into())),
    }
}

/// Revoke a key [`settle_key`] minted for a setting that was then not kept.
pub fn unmint(hub: &HubHandle, minted: Option<u32>) {
    if let (Some(cid), Some(auth)) = (minted, hub.inner.services.auth()) {
        auth.revoke(cid);
    }
}

// ── Shared with the service and the client's thread ─────────────────────────

#[derive(Debug, Default)]
struct Inbound {
    queue: VecDeque<(String, Vec<u8>)>,
    dropped: u64,
}

/// The `ha` setting as the pump and the worker see it, and the bounded queue of
/// `set` messages the client's thread fills.
#[derive(Debug)]
pub struct HaShared {
    setting: Mutex<HaSetting>,
    inbound: Mutex<Inbound>,
}

impl HaShared {
    /// With the setting read from `mqtt.json`.
    pub fn new(setting: HaSetting) -> Arc<Self> {
        Arc::new(Self {
            setting: Mutex::new(setting),
            inbound: Mutex::new(Inbound::default()),
        })
    }

    /// The setting now.
    pub fn setting(&self) -> HaSetting {
        *lock(&self.setting)
    }

    /// A new setting (`mqtt.set`); the pump follows it on its next pass.
    pub fn set(&self, setting: HaSetting) {
        *lock(&self.setting) = setting;
    }

    /// On the client's thread: a message goes on the bounded queue, unless Home
    /// Assistant is off, it was retained (a write is never replayed), or its topic or
    /// payload is too long. Never blocks but on this leaf lock.
    pub fn offer(&self, event: &MqttEvent) {
        let MqttEvent::Message {
            topic,
            payload,
            retain,
        } = event
        else {
            return;
        };
        if *retain || !self.setting().enabled {
            return;
        }
        let mut q = lock(&self.inbound);
        if topic.len() > MAX_TOPIC || payload.len() > MAX_PAYLOAD {
            q.dropped = q.dropped.saturating_add(1);
            return;
        }
        while q.queue.len() >= INBOUND_CAP {
            q.queue.pop_front();
            q.dropped = q.dropped.saturating_add(1);
        }
        q.queue.push_back((topic.clone(), payload.clone()));
    }

    /// Every waiting message, oldest first.
    pub fn take(&self) -> Vec<(String, Vec<u8>)> {
        lock(&self.inbound).queue.drain(..).collect()
    }

    /// Messages dropped: too long, or past [`INBOUND_CAP`].
    pub fn dropped(&self) -> u64 {
        lock(&self.inbound).dropped
    }
}

// ── The worker ──────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Jobs {
    queue: VecDeque<Job>,
    stop: bool,
    dropped: u64,
}

#[derive(Debug, Default)]
struct WorkerShared {
    jobs: Mutex<Jobs>,
    ready: Condvar,
    feedback: Mutex<VecDeque<Feedback>>,
}

impl WorkerShared {
    fn feed(&self, f: Feedback) {
        let mut q = lock(&self.feedback);
        while q.len() >= FEEDBACK_CAP {
            q.pop_front();
        }
        q.push_back(f);
    }
}

/// Whether `a` makes `b` (still waiting) needless: the same refresh, or a newer set of
/// the same entity.
fn supersedes(a: &Job, b: &Job) -> bool {
    match (a, b) {
        (Job::Scan, Job::Scan) => true,
        (Job::Refresh(x), Job::Refresh(y)) => x == y,
        (
            Job::Write {
                serial: s1,
                entity: e1,
                ..
            },
            Job::Write {
                serial: s2,
                entity: e2,
                ..
            },
        ) => s1 == s2 && e1 == e2 && *e1 != Entity::Identify,
        _ => false,
    }
}

/// The `hub-ha` thread: runs the jobs that block on a device.
pub struct Worker {
    shared: Arc<WorkerShared>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Start the thread. If it cannot start, jobs are dropped (logged once).
    pub fn start(hub: HubHandle, ha: Arc<HaShared>) -> Worker {
        let shared = Arc::new(WorkerShared::default());
        let runner = Runner {
            hub,
            ha,
            shared: Arc::clone(&shared),
            session: None,
            next_id: 1,
        };
        let thread = match thread::Builder::new()
            .name("hub-ha".into())
            .spawn(move || runner.run())
        {
            Ok(t) => Some(t),
            Err(e) => {
                log::error!("hub: the Home Assistant worker did not start: {e}");
                None
            }
        };
        Worker { shared, thread }
    }

    /// Queue a job; a newer one replaces the same job still waiting.
    pub fn push(&self, job: Job) {
        if self.thread.is_none() {
            return;
        }
        let mut jobs = lock(&self.shared.jobs);
        jobs.queue.retain(|old| !supersedes(&job, old));
        while jobs.queue.len() >= JOB_CAP {
            jobs.queue.pop_front();
            jobs.dropped = jobs.dropped.saturating_add(1);
        }
        jobs.queue.push_back(job);
        drop(jobs);
        self.shared.ready.notify_all();
    }

    /// What the worker found since last asked.
    pub fn take_feedback(&self) -> Vec<Feedback> {
        lock(&self.shared.feedback).drain(..).collect()
    }

    /// Jobs dropped past [`JOB_CAP`].
    pub fn dropped(&self) -> u64 {
        lock(&self.shared.jobs).dropped
    }

    /// Stop the thread and wait for it (at most the job it is in).
    pub fn stop(&mut self) {
        lock(&self.shared.jobs).stop = true;
        self.shared.ready.notify_all();
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            log::error!("hub: the Home Assistant worker panicked");
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A refusal of a call: its code and its line.
type Refusal = (String, String);

struct Runner {
    hub: HubHandle,
    ha: Arc<HaShared>,
    shared: Arc<WorkerShared>,
    /// The key's principal: its `cid` and session.
    session: Option<(u32, Arc<Session>)>,
    next_id: u64,
}

fn platform_name(platform: u8) -> &'static str {
    match platform {
        0 => "RP2040",
        1 => "RP2350",
        2 => "STM32",
        _ => "",
    }
}

impl Runner {
    fn run(mut self) {
        loop {
            let job = {
                let mut jobs = lock(&self.shared.jobs);
                loop {
                    if jobs.stop || self.hub.is_stopped() {
                        drop(jobs);
                        self.close();
                        return;
                    }
                    if let Some(job) = jobs.queue.pop_front() {
                        break job;
                    }
                    jobs = self
                        .shared
                        .ready
                        .wait_timeout(jobs, WORKER_WAIT)
                        .map(|(j, _)| j)
                        .unwrap_or_else(|e| e.into_inner().0);
                }
            };
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                Job::Scan => self.scan(),
                Job::Refresh(serial) => self.refresh(&serial),
                Job::Write {
                    serial,
                    entity,
                    msg,
                } => self.write(&serial, entity, msg),
            }));
            if ran.is_err() {
                log::error!("hub: a Home Assistant job panicked");
            }
        }
    }

    fn close(&mut self) {
        if let Some((_, s)) = self.session.take() {
            self.hub.close_session(s.id());
        }
    }

    /// The key's principal session, opened if needed, with the key's role now.
    fn principal(&mut self) -> Result<Arc<Session>, String> {
        let Some(cid) = self.ha.setting().key else {
            self.close();
            return Err("no API key is set for Home Assistant".into());
        };
        let Some(auth) = self.hub.inner.services.auth().cloned() else {
            return Err("this hub has no API keys".into());
        };
        let client = auth
            .clients()
            .into_iter()
            .find(|c| c.cid == cid && c.kind == ClientKind::Api);
        let Some(client) = client else {
            self.close();
            return Err(format!(
                "API key {cid} is revoked or is no API key: Home Assistant writes nothing"
            ));
        };
        if let Some((c, s)) = &self.session
            && *c == cid
            && !s.outbox().is_closed()
        {
            s.set_role(client.role);
            return Ok(Arc::clone(s));
        }
        self.close();
        let session = self
            .hub
            .open_principal(client.role, Some(cid), &client.name)
            .map_err(|e| e.to_string())?;
        // Nobody reads its outbox: it hears nothing (the executor's child sessions get
        // what they need through their own transports), as a schedule's principal.
        self.hub.inner.relay.remove_session(session.id());
        // Listed first, then asked, as REST does: a revoke in between is seen here.
        match auth.role_of(cid) {
            Some(role) => session.set_role(role),
            None => {
                self.hub.close_session(session.id());
                return Err(format!("API key {cid} was revoked"));
            }
        }
        self.session = Some((cid, Arc::clone(&session)));
        Ok(session)
    }

    /// `msg` through the control plane as `session`: its `ok` answer, or the refusal.
    fn ask(&mut self, session: &Arc<Session>, mut msg: Value) -> Result<Value, Refusal> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Some(map) = msg.as_object_mut() {
            map.insert("id".into(), id.into());
        }
        let Some(text) = crate::server::control_plane(&self.hub, session, &msg) else {
            return Err((
                "unknown_type".into(),
                "this hub does not run that family".into(),
            ));
        };
        let answer: Value = serde_json::from_str(&text)
            .map_err(|e| ("internal".to_owned(), format!("an unreadable answer: {e}")))?;
        if answer.get("t").and_then(Value::as_str) == Some("ok") {
            return Ok(answer);
        }
        let field = |k: &str| {
            answer
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        Err((field("code"), field("msg")))
    }

    fn scan(&mut self) {
        let online: Vec<String> = self
            .hub
            .devices()
            .into_iter()
            .map(|d| {
                let online = d.state == DeviceState::Online;
                let serial = d.identity.serial.clone();
                self.shared.feed(Feedback::Device {
                    serial: serial.clone(),
                    name: d.name,
                    fw: d.identity.firmware,
                    platform: platform_name(d.identity.platform).to_owned(),
                    online,
                });
                (serial, online)
            })
            .filter(|(_, online)| *online)
            .map(|(s, _)| s)
            .collect();
        for serial in online {
            self.values(&serial);
        }
    }

    fn refresh(&mut self, serial: &str) {
        let Some(d) = self
            .hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial)
        else {
            return;
        };
        let online = d.state == DeviceState::Online;
        self.shared.feed(Feedback::Device {
            serial: serial.to_owned(),
            name: d.name,
            fw: d.identity.firmware,
            platform: platform_name(d.identity.platform).to_owned(),
            online,
        });
        if online {
            self.values(serial);
        }
    }

    /// Read the volume, the mute and the presets as the key's principal.
    fn values(&mut self, serial: &str) {
        let session = match self.principal() {
            Ok(s) => s,
            Err(why) => {
                log::debug!("hub: Home Assistant cannot read {serial}: {why}");
                return;
            }
        };
        let get = |path: &str| json!({"t": "param.get", "serial": serial, "path": path, "idx": []});
        let volume = self
            .ask(&session, get("vol.master"))
            .ok()
            .and_then(|v| v.get("value").and_then(Value::as_f64));
        let mute = self
            .ask(&session, get("vol.mute"))
            .ok()
            .and_then(|v| v.get("value").and_then(Value::as_bool));
        let list = self
            .ask(&session, json!({"t": "preset.list", "serial": serial}))
            .ok();
        let (presets, active) = match list {
            Some(list) => {
                let slots: Vec<(u8, String, bool)> = list
                    .get("slots")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .take(PRESET_SLOTS)
                            .filter_map(|r| {
                                let slot = u8::try_from(r.get("slot")?.as_u64()?).ok()?;
                                let name = r.get("name")?.as_str()?.to_owned();
                                let used = r.get("used")?.as_bool()?;
                                Some((slot, name, used))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let active = list
                    .get("active")
                    .and_then(Value::as_u64)
                    .and_then(|a| u8::try_from(a).ok());
                (Some(preset_labels(&slots)), Some(active))
            }
            None => (None, None),
        };
        self.shared.feed(Feedback::Values {
            serial: serial.to_owned(),
            volume,
            mute,
            presets,
            active,
        });
    }

    fn write(&mut self, serial: &str, entity: Entity, msg: Value) {
        let refused = |me: &Self, code: &str, why: &str| {
            log::info!(
                "hub: a Home Assistant {} set on {serial} was refused ({code}): {why}",
                entity.word()
            );
            me.shared.feed(Feedback::Refused {
                serial: serial.to_owned(),
                entity,
            });
        };
        let session = match self.principal() {
            Ok(s) => s,
            Err(why) => return refused(self, "unauthenticated", &why),
        };
        match self.ask(&session, msg) {
            Ok(answer) => match entity {
                Entity::MasterVolume => self.shared.feed(Feedback::Values {
                    serial: serial.to_owned(),
                    volume: answer.get("value").and_then(Value::as_f64),
                    mute: None,
                    presets: None,
                    active: None,
                }),
                Entity::Mute => self.shared.feed(Feedback::Values {
                    serial: serial.to_owned(),
                    volume: None,
                    mute: answer.get("value").and_then(Value::as_bool),
                    presets: None,
                    active: None,
                }),
                Entity::Preset => {
                    let slot = answer
                        .get("slot")
                        .and_then(Value::as_u64)
                        .and_then(|s| u8::try_from(s).ok());
                    self.shared.feed(Feedback::Values {
                        serial: serial.to_owned(),
                        volume: None,
                        mute: None,
                        presets: None,
                        active: slot.map(Some),
                    });
                    // The preset's volume and mute.
                    self.values(serial);
                }
                _ => {}
            },
            Err((code, why)) => refused(self, &code, &why),
        }
    }
}

// ── In P2MQ's pump ──────────────────────────────────────────────────────────

/// The bus messages Home Assistant follows.
fn wanted(m: &BusMessage) -> bool {
    matches!(
        m,
        BusMessage::DeviceOnline { .. }
            | BusMessage::DeviceOffline { .. }
            | BusMessage::DeviceRemoved { .. }
            | BusMessage::Notification { .. }
            | BusMessage::Health { .. }
            | BusMessage::Facts { .. }
    )
}

/// While Home Assistant is on: its state, its bus subscription and its worker.
struct On {
    state: HaState,
    sub: Subscription,
    worker: Worker,
    subscribed: bool,
}

/// Home Assistant inside one MQTT connection's pump (P2MQ's `service.rs`). It starts
/// and stops with the `ha` setting, without a reconnect.
pub struct HaRuntime {
    shared: Arc<HaShared>,
    hub: HubHandle,
    bus: Arc<Bus>,
    base: String,
    hub_id: String,
    out: Outbox,
    on: Option<On>,
    /// The inbound drop count last logged.
    logged_drops: u64,
}

impl HaRuntime {
    /// For a connection with base topic `base`.
    pub fn new(shared: Arc<HaShared>, hub: HubHandle, bus: Arc<Bus>, base: String) -> Self {
        let hub_id = hub.hub_id().to_owned();
        Self {
            shared,
            hub,
            bus,
            base,
            hub_id,
            out: Outbox::default(),
            on: None,
            logged_drops: 0,
        }
    }

    /// The broker accepted a connection: subscribe (the client keeps it across
    /// reconnects) and publish everything again.
    pub fn on_connected(&mut self, client: &MqttClient) {
        if let Some(on) = &mut self.on {
            Self::subscribe(&self.base, on, client);
            on.state.republish_all(&mut self.out);
        }
    }

    fn subscribe(base: &str, on: &mut On, client: &MqttClient) {
        if on.subscribed {
            return;
        }
        match client.subscribe(&set_filter(base), QoS::AtLeastOnce) {
            Ok(()) => on.subscribed = true,
            Err(e) => log::warn!("hub: Home Assistant's set topics not subscribed: {e}"),
        }
    }

    /// One pump pass: follow the setting, take the `set` messages, the bus and the
    /// worker's findings, and publish what has room.
    pub fn tick(&mut self, client: &MqttClient) {
        let enabled = self.shared.setting().enabled;
        if enabled && self.on.is_none() {
            let mut on = On {
                state: HaState::new(&self.base, &self.hub_id),
                sub: self.bus.subscribe_filtered("mqtt-ha", BUS_CAPACITY, wanted),
                worker: Worker::start(self.hub.clone(), Arc::clone(&self.shared)),
                subscribed: false,
            };
            on.worker.push(Job::Scan);
            if client.status().connected {
                Self::subscribe(&self.base, &mut on, client);
            }
            self.on = Some(on);
        } else if !enabled && let Some(mut on) = self.on.take() {
            on.worker.stop();
            on.state.remove_all(&mut self.out);
            // Set messages already queued are not run.
            let _ = self.shared.take();
        }
        if let Some(on) = &mut self.on {
            for (topic, payload) in self.shared.take() {
                match parse_set(&self.base, &topic, &payload) {
                    Ok((serial, entity, command)) => {
                        if let Some(job) = on.state.command(&mut self.out, &serial, entity, command)
                        {
                            on.worker.push(job);
                        }
                    }
                    Err(e) => log::debug!("hub: a Home Assistant message dropped: {e}"),
                }
            }
            for _ in 0..BUS_BATCH {
                let Some(m) = on.sub.try_recv() else { break };
                for job in on.state.on_bus(&mut self.out, &m) {
                    on.worker.push(job);
                }
            }
            if on.sub.take_dropped() > 0 {
                // A change may be lost: read every device again.
                on.worker.push(Job::Scan);
            }
            for f in on.worker.take_feedback() {
                on.state.on_feedback(&mut self.out, f);
            }
            let dropped = self.shared.dropped();
            if dropped > self.logged_drops {
                log::warn!(
                    "hub: Home Assistant dropped {} set message(s) (too long or too many)",
                    dropped.saturating_sub(self.logged_drops)
                );
                self.logged_drops = dropped;
            }
        }
        self.flush(client);
    }

    fn flush(&mut self, client: &MqttClient) {
        for _ in 0..FLUSH_BATCH {
            if self.out.is_empty() || client.status().queued >= FLUSH_BELOW {
                return;
            }
            let Some((topic, payload)) = self.out.pop() else {
                return;
            };
            if let Err(e) = client.publish(&topic, &payload, QoS::AtLeastOnce, true) {
                log::warn!("hub: a Home Assistant publish to {topic} was refused: {e}");
            }
        }
    }

    /// The connection ends: stop the worker. Retained entities stay; the hub's will
    /// and `offline` status make them unavailable.
    pub fn end(&mut self, client: &MqttClient) {
        self.flush(client);
        if let Some(mut on) = self.on.take() {
            on.worker.stop();
        }
    }
}

#[cfg(test)]
#[path = "tests_wp_p2ha.rs"]
mod tests_wp_p2ha;
