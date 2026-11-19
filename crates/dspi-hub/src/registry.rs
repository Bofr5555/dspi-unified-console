//! Devices by handle and serial (spec 7.4). Owned by WP HB3.
//!
//! Pure: the caller passes the clock, and every change returns its event so the
//! caller publishes it with the hub's lock released, and saves the stored names the
//! same way (the disk is no place for the hub's lock).
//!
//! **Names (spec 7.4).** A board's own name wins, then the hub-stored name, then a
//! default from the serial, applied each time the device comes online. A board that can
//! store a name has it written there, and only its `NOTIFY_EVT_DEVICE_NAME` changes the
//! name here, so a rename from any source is seen. A stored name for an unnamed board
//! that can hold one is written to it once, and goes as soon as the board accepts that,
//! or reports a name of its own, or notifies at all: a later clear is never undone.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use serde_json::{Value, json};

use crate::error::HubError;
use crate::session::SessionId;

/// How long a device that dropped keeps its handle (Swift hub; spec 8.2 asks for
/// 60 s after a bootloader entry).
pub const OFFLINE_GRACE: Duration = Duration::from_secs(60);
/// Handles are 0..=254; 255 is reserved (spec 2).
const MAX_HANDLE: u8 = 254;
/// Where a `NOTIFY_EVT_DEVICE_NAME`'s name begins.
const BOARD_NAME_FIELD: usize = 8;

/// The longest name a board stores (`DEVICE_NAME_LEN` less its NUL).
const BOARD_NAME_MAX: usize = 31;
/// The longest name kept for a board that cannot store one, in characters. A client's
/// say-so, kept on disk and listed in UIs.
const STORED_NAME_MAX: usize = 64;

/// Where hub-stored device names persist (WP HB1's store), by serial. Both calls come
/// with no hub lock held, one at a time.
pub trait NameStore: Send + Sync {
    /// The names kept from last time. Called once, at start.
    fn load(&self) -> BTreeMap<String, String>;
    /// Replace the names kept. A failure costs them at the next start, not this run.
    fn save(&self, names: &BTreeMap<String, String>) -> Result<(), HubError>;
}

/// A name as the board can store it: printable ASCII with the gaps left by dropped
/// characters closed, ends trimmed, at most 31 characters. "" clears (spec 7.4).
pub fn board_name_from(name: &str) -> String {
    let ascii: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .collect();
    let joined = ascii.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(BOARD_NAME_MAX).collect();
    cut.trim_end().to_owned()
}

/// What goes on the wire for a board name: the 32-byte field, NUL-padded, so "" clears.
pub(crate) fn board_name_payload(name: &str) -> Vec<u8> {
    let mut field = name.as_bytes().to_vec();
    field.resize(BOARD_NAME_MAX.saturating_add(1), 0);
    field
}

/// The name in a device-name field: UTF-8, up to the first NUL, at most 31 bytes.
pub(crate) fn decode_board_name(field: &[u8]) -> String {
    let field = field.get(..field.len().min(BOARD_NAME_MAX)).unwrap_or(&[]);
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(field.get(..end).unwrap_or(&[])).into_owned()
}

/// The name a `NOTIFY_EVT_DEVICE_NAME` carries: `[2, evt, 0, seq, id, src, 0, 0,
/// name(32)]`. `None` for any other packet, or one too short to hold the field.
pub(crate) fn board_name_event(packet: &[u8]) -> Option<String> {
    if packet.get(1).copied() != Some(nfy::NOTIFY_EVT_DEVICE_NAME as u8) {
        return None;
    }
    let end = BOARD_NAME_FIELD
        .saturating_add(BOARD_NAME_MAX)
        .saturating_add(1);
    packet.get(BOARD_NAME_FIELD..end).map(decode_board_name)
}

/// What the hub publishes about a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// The 16-character serial.
    pub serial: String,
    /// USB vendor id (current or legacy firmware), 0 for a non-USB link.
    pub vid: u16,
    /// Where it sits on the bus (`dspi_core::DeviceInfo::location_id`), 0 if nowhere.
    pub location_id: u32,
    /// 0 RP2040, 1 RP2350, 2 STM32.
    pub platform: u8,
    /// "1.1.7", without the beta.
    pub firmware: String,
    /// The beta number; `None` if the reply is too short to say.
    pub firmware_beta: Option<u8>,
    /// Output channels.
    pub outputs: u8,
    /// The board's own name: `None` if the firmware cannot store one, empty if
    /// unnamed.
    pub board_name: Option<String>,
    // -- WP PGHDEV --
    /// Byte 7 of the platform read (8 bytes asked): the board id, 0 when the firmware
    /// answers fewer bytes (PLAN-PG section 1.9). The router refuses a bootloader entry
    /// toward any board but 0.
    pub board: u8,
    /// The vendor protocol version (`major`, `minor`) of a paged (1.1.7) device, from
    /// its attach probe; `None` for a legacy one. Read again at every attach and only
    /// shown: what the hub forwards is decided by the live transport's generation
    /// (`DeviceShared::protocol`), never by this record.
    pub protocol: Option<(u8, u8)>,
    /// Whether a paged device has our extension channel (page `0xE0` in its
    /// `SYS.PAGE_LIST`); `None` for a legacy device (its clients probe `EXT_LIST`
    /// themselves, as before) and when the read at attach did not say.
    pub ext: Option<bool>,
}

/// A device's state (spec 7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Attached and answering.
    Online,
    /// Dropped; its handle is kept for the grace period.
    Offline,
}

/// One device the hub knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredDevice {
    /// Its handle.
    pub handle: u8,
    /// Its identity.
    pub identity: DeviceIdentity,
    /// The name clients show.
    pub name: String,
    /// Online or offline.
    pub state: DeviceState,
    /// The session holding its lock.
    pub locked_by: Option<SessionId>,
}

/// A change to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryEvent {
    /// A new device.
    Added(RegisteredDevice),
    /// Its state or identity changed.
    Changed(RegisteredDevice),
    /// Its grace ended.
    Removed {
        /// The freed handle.
        handle: u8,
        /// The serial.
        serial: String,
    },
}

impl RegisteredDevice {
    /// The device object of spec 7.4, as in `device.list` and the events. `fw_beta`
    /// is left out when unknown, and so are `inputs` and `wire_version`, which a hub
    /// that does not interpret the image has no reason to know (a client reads them).
    /// `link` is `usb`: the spec's other value is a bridge's UART.
    pub fn to_json(&self) -> Value {
        let id = &self.identity;
        let mut device = json!({
            "handle": self.handle,
            "serial": id.serial,
            "name": self.name,
            "platform": id.platform,
            "fw": id.firmware,
            "outputs": id.outputs,
            "state": match self.state {
                DeviceState::Online => "online",
                DeviceState::Offline => "offline",
            },
            "link": "usb",
            "locked_by": self.locked_by,
        });
        if let (Some(beta), Some(map)) = (id.firmware_beta, device.as_object_mut()) {
            map.insert("fw_beta".into(), beta.into());
        }
        // -- WP PGHDEV -- spec 1.3 section 7.4 for a paged device; absent for a
        // legacy one, which this hub still lists and forwards to (OD-PGg, cap
        // `legacy_devices`); `ext` is ours, additive
        if let Some(map) = device.as_object_mut() {
            if let Some((major, minor)) = id.protocol {
                map.insert("protocol".into(), json!({"major": major, "minor": minor}));
            }
            if let Some(ext) = id.ext {
                map.insert("ext".into(), ext.into());
            }
        }
        device
    }
}

impl RegistryEvent {
    /// The control-plane message for this change (`device.added` and so on, spec 7.4).
    pub fn to_json(&self) -> String {
        match self {
            RegistryEvent::Added(d) => json!({"t": "device.added", "device": d.to_json()}),
            RegistryEvent::Changed(d) => json!({"t": "device.changed", "device": d.to_json()}),
            RegistryEvent::Removed { handle, serial } => {
                json!({"t": "device.removed", "handle": handle, "serial": serial})
            }
        }
        .to_string()
    }
}

/// What [`Registry::online`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Online {
    /// The device's handle: its old one if it came back within the grace.
    pub handle: u8,
    /// It came back: clients get a RESYNC (reason 1).
    pub returning: bool,
    /// The event to publish.
    pub event: RegistryEvent,
    /// A stored name to write to this unnamed board, once.
    pub adopt: Option<String>,
    /// The stored names changed, so save them.
    pub names_changed: bool,
}

/// What [`Registry::board_name_stored`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardNamed {
    /// The event to publish, if the name changed.
    pub event: Option<RegistryEvent>,
    /// The stored names changed, so save them.
    pub names_changed: bool,
}

#[derive(Debug, Clone)]
struct Entry {
    device: RegisteredDevice,
    offline_since: Option<Instant>,
}

/// The hub's devices, keyed by serial.
#[derive(Debug)]
pub struct Registry {
    grace: Duration,
    by_serial: BTreeMap<String, Entry>,
    /// Hub-stored names, by serial. They outlive a device's entry.
    stored: BTreeMap<String, String>,
}

impl Registry {
    /// An empty registry with this offline grace.
    pub fn new(grace: Duration) -> Self {
        Self {
            grace,
            by_serial: BTreeMap::new(),
            stored: BTreeMap::new(),
        }
    }

    /// A registry that starts with these stored names.
    pub fn with_names(grace: Duration, stored: BTreeMap<String, String>) -> Self {
        Self {
            stored,
            ..Self::new(grace)
        }
    }

    /// The stored names, to save.
    pub fn stored_names(&self) -> &BTreeMap<String, String> {
        &self.stored
    }

    /// Apply the order of spec 7.4 as `identity` comes online: its name, the stored
    /// name to write to it, and whether a stored entry retired.
    fn resolve(&mut self, identity: &DeviceIdentity) -> (String, Option<String>, bool) {
        if let Some(board) = identity.board_name.as_deref().filter(|n| !n.is_empty()) {
            let retired = self.stored.remove(&identity.serial).is_some();
            return (board.to_owned(), None, retired);
        }
        let stored = self.stored.get(&identity.serial);
        let adopt = stored
            .filter(|_| identity.board_name.is_some())
            .map(|name| board_name_from(name))
            .filter(|name| !name.is_empty());
        let name = stored
            .cloned()
            .unwrap_or_else(|| default_name(&identity.serial));
        (name, adopt, false)
    }

    /// The device is present. `None` when no handle is free.
    pub fn online(&mut self, identity: DeviceIdentity) -> Option<Online> {
        let known = self.by_serial.contains_key(&identity.serial);
        if !known {
            self.free_handle()?;
        }
        let (name, adopt, names_changed) = self.resolve(&identity);
        if let Some(entry) = self.by_serial.get_mut(&identity.serial) {
            entry.offline_since = None;
            entry.device.name = name;
            entry.device.identity = identity;
            entry.device.state = DeviceState::Online;
            return Some(Online {
                handle: entry.device.handle,
                returning: true,
                event: RegistryEvent::Changed(entry.device.clone()),
                adopt,
                names_changed,
            });
        }
        let handle = self.free_handle()?;
        let device = RegisteredDevice {
            handle,
            name,
            identity,
            state: DeviceState::Online,
            locked_by: None,
        };
        self.by_serial.insert(
            device.identity.serial.clone(),
            Entry {
                device: device.clone(),
                offline_since: None,
            },
        );
        Some(Online {
            handle,
            returning: false,
            event: RegistryEvent::Added(device),
            adopt,
            names_changed,
        })
    }

    /// The device dropped. Its lock goes with it; its handle stays for the grace.
    pub fn offline(&mut self, serial: &str, now: Instant) -> Option<RegistryEvent> {
        let entry = self.by_serial.get_mut(serial)?;
        if entry.device.state == DeviceState::Offline {
            return None;
        }
        entry.device.state = DeviceState::Offline;
        entry.device.locked_by = None;
        entry.offline_since = Some(now);
        Some(RegistryEvent::Changed(entry.device.clone()))
    }

    /// Forget devices whose grace has ended.
    pub fn reap(&mut self, now: Instant) -> Vec<RegistryEvent> {
        let grace = self.grace;
        let expired: Vec<String> = self
            .by_serial
            .iter()
            .filter(|(_, e)| {
                e.offline_since
                    .is_some_and(|t| now.saturating_duration_since(t) >= grace)
            })
            .map(|(s, _)| s.clone())
            .collect();
        expired
            .into_iter()
            .filter_map(|serial| {
                let entry = self.by_serial.remove(&serial)?;
                Some(RegistryEvent::Removed {
                    handle: entry.device.handle,
                    serial,
                })
            })
            .collect()
    }

    // -- WP HB2 --
    /// Who holds the lock of the online device at `handle`, as the router says. An
    /// event if that changed. An offline device has none (see [`Registry::offline`]).
    pub fn set_locked_by(&mut self, handle: u8, by: Option<SessionId>) -> Option<RegistryEvent> {
        let entry = self
            .by_serial
            .values_mut()
            .find(|e| e.device.handle == handle && e.device.state == DeviceState::Online)?;
        if entry.device.locked_by == by {
            return None;
        }
        entry.device.locked_by = by;
        Some(RegistryEvent::Changed(entry.device.clone()))
    }

    /// `device.rename` for a board that cannot store a name (spec 7.4): the name is
    /// kept here, and "" goes back to the default. A board that can is written instead
    /// ([`crate::HubHandle::rename`]), which this must not be used for: the board's own
    /// name would override this the next time it comes online. `None` for no such
    /// handle. The stored names changed, so save them.
    pub fn rename(&mut self, handle: u8, name: &str) -> Option<RegistryEvent> {
        let entry = self
            .by_serial
            .values_mut()
            .find(|e| e.device.handle == handle)?;
        let serial = entry.device.identity.serial.clone();
        let clean: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(STORED_NAME_MAX)
            .collect();
        let clean = clean.trim();
        if clean.is_empty() {
            entry.device.name = default_name(&serial);
            self.stored.remove(&serial);
        } else {
            entry.device.name = clean.to_owned();
            self.stored.insert(serial, clean.to_owned());
        }
        Some(RegistryEvent::Changed(entry.device.clone()))
    }

    /// The board stored a name (`NOTIFY_EVT_DEVICE_NAME`), whoever asked. On such a
    /// board this is the only thing that sets the name, and it retires the stored entry
    /// so a later clear is never undone.
    pub fn board_name_stored(&mut self, serial: &str, name: &str) -> BoardNamed {
        let names_changed = self.stored.remove(serial).is_some();
        let Some(entry) = self.by_serial.get_mut(serial) else {
            return BoardNamed {
                event: None,
                names_changed,
            };
        };
        entry.device.identity.board_name = Some(name.to_owned());
        let resolved = if name.is_empty() {
            default_name(serial)
        } else {
            name.to_owned()
        };
        let changed = entry.device.name != resolved;
        entry.device.name = resolved;
        BoardNamed {
            event: changed.then(|| RegistryEvent::Changed(entry.device.clone())),
            names_changed,
        }
    }

    /// The board accepted the stored name's write, so that entry is done with, even if
    /// the notification that follows is lost. Whether the stored names changed.
    pub fn adoption_accepted(&mut self, serial: &str) -> bool {
        self.stored.remove(serial).is_some()
    }

    /// The device with this handle.
    pub fn by_handle(&self, handle: u8) -> Option<&RegisteredDevice> {
        self.by_serial
            .values()
            .map(|e| &e.device)
            .find(|d| d.handle == handle)
    }

    /// Every device, by handle.
    pub fn devices(&self) -> Vec<RegisteredDevice> {
        let mut all: Vec<_> = self.by_serial.values().map(|e| e.device.clone()).collect();
        all.sort_by_key(|d| d.handle);
        all
    }

    /// The device with this serial.
    pub fn by_serial(&self, serial: &str) -> Option<&RegisteredDevice> {
        self.by_serial.get(serial).map(|e| &e.device)
    }

    /// Lowest handle not in use. An offline device keeps its own, so a replacement
    /// board plugged in during the grace gets the next one.
    fn free_handle(&self) -> Option<u8> {
        (0..=MAX_HANDLE).find(|h| !self.by_serial.values().any(|e| e.device.handle == *h))
    }
}

/// "DSPi" and the serial's last six characters.
fn default_name(serial: &str) -> String {
    let n = serial.chars().count();
    let tail: String = serial.chars().skip(n.saturating_sub(6)).collect();
    format!("DSPi {tail}")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use super::*;

    fn identity(serial: &str) -> DeviceIdentity {
        DeviceIdentity {
            serial: serial.into(),
            vid: 0,
            location_id: 0,
            platform: 1,
            firmware: "1.1.7".into(),
            firmware_beta: Some(0),
            outputs: 9,
            board_name: None,
            board: 0,
            protocol: None,
            ext: None,
        }
    }

    #[test]
    fn wp_hb0_a_device_back_within_the_grace_keeps_its_handle() {
        let t = Instant::now();
        let mut r = Registry::new(Duration::from_secs(60));
        let a = r.online(identity("A")).unwrap();
        assert!(!a.returning);
        let b = r.online(identity("B")).unwrap();
        assert_ne!(a.handle, b.handle);
        r.offline("A", t).unwrap();
        assert_eq!(r.offline("A", t), None, "only once");
        // A replacement board during the grace does not take the handle.
        let c = r.online(identity("C")).unwrap();
        assert_ne!(c.handle, a.handle);
        let back = r.online(identity("A")).unwrap();
        assert_eq!(back.handle, a.handle);
        assert!(back.returning);
    }

    #[test]
    fn wp_hb0_the_handle_goes_after_the_grace() {
        let t = Instant::now();
        let grace = Duration::from_secs(60);
        let mut r = Registry::new(grace);
        let a = r.online(identity("A")).unwrap();
        r.offline("A", t).unwrap();
        assert!(r.reap(t + grace / 2).is_empty());
        let gone = r.reap(t + grace);
        assert_eq!(
            gone,
            [RegistryEvent::Removed {
                handle: a.handle,
                serial: "A".into()
            }]
        );
        assert!(!r.online(identity("A")).unwrap().returning);
    }

    #[test]
    fn wp_hb0_handles_run_out_at_255_devices() {
        let mut r = Registry::new(Duration::from_secs(60));
        for n in 0..=254 {
            assert_eq!(r.online(identity(&n.to_string())).unwrap().handle, n as u8);
        }
        assert_eq!(r.online(identity("one too many")), None);
    }

    /// Read back by the client this repo ships, field by field.
    #[test]
    fn wp_hb0_events_are_what_the_client_reads() {
        let t = Instant::now();
        let mut r = Registry::new(Duration::from_secs(60));
        let added = r.online(identity("E6614C311B8B4E3A")).unwrap().event;
        let RegistryEvent::Added(device) = &added else {
            panic!("{added:?}");
        };
        let parsed: dspi_link::DeviceInfo = serde_json::from_value(device.to_json()).unwrap();
        assert_eq!(parsed.handle, 0);
        assert_eq!(parsed.serial, "E6614C311B8B4E3A");
        assert_eq!(parsed.name, "DSPi 8B4E3A");
        assert_eq!(parsed.platform, Some(1));
        assert_eq!(parsed.fw.as_deref(), Some("1.1.7"));
        assert_eq!(parsed.fw_beta, Some(0));
        assert_eq!(parsed.outputs, Some(9));
        assert_eq!(parsed.state, "online");
        assert_eq!(parsed.link, "usb");
        assert_eq!(parsed.locked_by, None);

        let json: Value = serde_json::from_str(&added.to_json()).unwrap();
        assert_eq!(json["t"], "device.added");
        assert_eq!(json["device"]["locked_by"], Value::Null, "null, not absent");

        let changed = r.offline("E6614C311B8B4E3A", t).unwrap();
        let json: Value = serde_json::from_str(&changed.to_json()).unwrap();
        assert_eq!(json["t"], "device.changed");
        assert_eq!(json["device"]["state"], "offline");

        let gone = RegistryEvent::Removed {
            handle: 3,
            serial: "A".into(),
        };
        let json: Value = serde_json::from_str(&gone.to_json()).unwrap();
        assert_eq!(json["t"], "device.removed");
        assert_eq!(
            (json["handle"].as_u64(), json["serial"].as_str()),
            (Some(3), Some("A"))
        );
    }

    #[test]
    fn wp_hb0_an_unknown_beta_is_left_out() {
        let mut r = Registry::new(Duration::from_secs(60));
        let mut id = identity("A");
        id.firmware_beta = None;
        let RegistryEvent::Added(d) = r.online(id).unwrap().event else {
            panic!();
        };
        assert!(d.to_json().get("fw_beta").is_none());
    }

    #[test]
    fn wp_hb0_a_serial_of_any_length_gets_a_name() {
        assert_eq!(default_name("E6614C311B8B4E3A"), "DSPi 8B4E3A");
        assert_eq!(default_name("ab"), "DSPi ab");
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb3 {
    use super::*;

    const GRACE: Duration = Duration::from_secs(60);

    fn identity(serial: &str, board: Option<&str>) -> DeviceIdentity {
        DeviceIdentity {
            serial: serial.into(),
            vid: 0,
            location_id: 0,
            platform: 1,
            firmware: "1.1.7".into(),
            firmware_beta: Some(0),
            outputs: 9,
            board_name: board.map(str::to_owned),
            board: 0,
            protocol: None,
            ext: None,
        }
    }

    fn stored(serial: &str, name: &str) -> Registry {
        Registry::with_names(
            GRACE,
            BTreeMap::from([(serial.to_owned(), name.to_owned())]),
        )
    }

    fn name_of(r: &Registry, serial: &str) -> String {
        r.by_serial(serial).unwrap().name.clone()
    }

    #[test]
    fn wp_hb3_the_board_name_beats_the_stored_name_and_retires_it() {
        let mut r = stored("A", "Kept");
        let on = r.online(identity("A", Some("Board"))).unwrap();
        assert_eq!(name_of(&r, "A"), "Board");
        assert!(on.names_changed);
        assert_eq!(on.adopt, None);
        assert!(r.stored_names().is_empty());
    }

    #[test]
    fn wp_hb3_older_firmware_shows_the_stored_name_and_never_adopts() {
        let mut r = stored("A", "Kept");
        let on = r.online(identity("A", None)).unwrap();
        assert_eq!(name_of(&r, "A"), "Kept");
        assert_eq!((on.adopt, on.names_changed), (None, false));
        assert_eq!(r.stored_names().len(), 1);
    }

    #[test]
    fn wp_hb3_an_unnamed_board_adopts_the_stored_name_as_it_can_hold_it() {
        let mut r = stored("A", " Vardagsrum  Köket ");
        let on = r.online(identity("A", Some(""))).unwrap();
        assert_eq!(on.adopt.as_deref(), Some("Vardagsrum Kket"));
        // Until the board has it, the stored name shows, and stays.
        assert_eq!(name_of(&r, "A"), " Vardagsrum  Köket ");
        assert_eq!(r.stored_names().len(), 1);
        assert!(r.adoption_accepted("A"));
        assert!(!r.adoption_accepted("A"));
        // Not adopted again when it returns, even if the board stayed unnamed.
        r.offline("A", Instant::now()).unwrap();
        let on = r.online(identity("A", Some(""))).unwrap();
        assert_eq!(on.adopt, None);
    }

    #[test]
    fn wp_hb3_a_stored_name_the_board_cannot_hold_is_not_written() {
        let mut r = stored("A", "Köö");
        assert_eq!(
            r.online(identity("A", Some(""))).unwrap().adopt.as_deref(),
            Some("K")
        );
        let mut r = stored("A", "ÅÄÖ");
        assert_eq!(r.online(identity("A", Some(""))).unwrap().adopt, None);
    }

    #[test]
    fn wp_hb3_the_default_comes_last() {
        let mut r = Registry::new(GRACE);
        r.online(identity("E6614C311B8B4E3A", Some(""))).unwrap();
        assert_eq!(name_of(&r, "E6614C311B8B4E3A"), "DSPi 8B4E3A");
    }

    #[test]
    fn wp_hb3_the_name_is_resolved_each_time_it_comes_online() {
        let t = Instant::now();
        let mut r = Registry::new(GRACE);
        r.online(identity("A", Some("One"))).unwrap();
        r.offline("A", t).unwrap();
        // Renamed on another computer while away.
        let back = r.online(identity("A", Some("Two"))).unwrap();
        assert!(back.returning);
        let RegistryEvent::Changed(d) = back.event else {
            panic!();
        };
        assert_eq!(d.name, "Two");
    }

    #[test]
    fn wp_hb3_a_rename_keeps_older_firmwares_name_and_clears_it() {
        let t = Instant::now();
        let mut r = Registry::new(GRACE);
        let on = r.online(identity("A", None)).unwrap();
        let RegistryEvent::Changed(d) = r.rename(on.handle, "Hall\n ").unwrap() else {
            panic!();
        };
        assert_eq!(d.name, "Hall");
        assert_eq!(r.stored_names()["A"], "Hall");
        // It comes back with it, past the grace too.
        r.offline("A", t).unwrap();
        r.reap(t + GRACE);
        assert!(r.by_serial("A").is_none());
        r.online(identity("A", None)).unwrap();
        assert_eq!(name_of(&r, "A"), "Hall");
        let h = r.by_serial("A").unwrap().handle;
        r.rename(h, "   ").unwrap();
        assert_eq!(name_of(&r, "A"), "DSPi A");
        assert!(r.stored_names().is_empty());
        assert_eq!(r.rename(200, "x"), None);
        let long = "x".repeat(1000);
        r.rename(h, &long).unwrap();
        assert_eq!(name_of(&r, "A").chars().count(), STORED_NAME_MAX);
    }

    #[test]
    fn wp_hb3_the_boards_notification_sets_the_name_and_retires_the_entry() {
        let mut r = stored("A", "Kept");
        r.online(identity("A", Some(""))).unwrap();
        let named = r.board_name_stored("A", "Den");
        assert!(named.names_changed);
        assert!(matches!(named.event, Some(RegistryEvent::Changed(ref d)) if d.name == "Den"));
        assert!(r.stored_names().is_empty());
        assert_eq!(r.board_name_stored("A", "Den").event, None, "no change");
        // A clear is a clear: the default, and no stored name comes back.
        assert_eq!(
            name_of(
                &{
                    r.board_name_stored("A", "");
                    r
                },
                "A"
            ),
            "DSPi A"
        );
        // Older firmware that turns out to name itself is a board from now on.
        let mut r = Registry::new(GRACE);
        r.online(identity("B", None)).unwrap();
        r.board_name_stored("B", "New");
        assert_eq!(
            r.by_serial("B").unwrap().identity.board_name.as_deref(),
            Some("New")
        );
        assert_eq!(r.board_name_stored("gone", "x"), BoardNamed::default());
    }

    #[test]
    fn wp_hb3_names_reduce_as_the_spec_says() {
        for (given, want) in [
            ("Living room", "Living room"),
            ("  Den  ", "Den"),
            ("A\tB", "AB"),
            ("Kö k", "K k"),
            ("Å Ä Ö x", "x"),
            ("", ""),
            ("Å", ""),
        ] {
            assert_eq!(board_name_from(given), want, "{given:?}");
        }
        let long = board_name_from(&"y".repeat(40));
        assert_eq!(long.len(), 31);
        // A cut that lands on a space does not leave it at the end.
        let spaced = format!("{} z", "w".repeat(30));
        assert_eq!(board_name_from(&spaced), "w".repeat(30));
    }

    #[test]
    fn wp_hb3_the_wire_field_is_32_bytes_and_empty_clears() {
        let p = board_name_payload("Den");
        assert_eq!(p.len(), 32);
        assert_eq!(&p[..4], b"Den\0");
        assert_eq!(board_name_payload(""), [0; 32]);
        assert_eq!(decode_board_name(&p), "Den");
        assert_eq!(decode_board_name(&[b'a'; 32]).len(), 31);
        assert_eq!(decode_board_name(&[]), "");
    }

    #[test]
    fn wp_hb3_only_a_whole_name_event_is_read() {
        let mut p = vec![2, nfy::NOTIFY_EVT_DEVICE_NAME as u8, 0, 0, 0, 0, 0, 0];
        p.extend_from_slice(&board_name_payload("Den"));
        assert_eq!(board_name_event(&p).as_deref(), Some("Den"));
        assert_eq!(board_name_event(&p[..39]), None, "too short");
        p[1] = nfy::NOTIFY_EVT_INPUT_SOURCE_NAME as u8;
        assert_eq!(board_name_event(&p), None, "a source's name");
        assert_eq!(board_name_event(&[]), None);
    }
}

// -- WP PGHDEV --
#[cfg(test)]
mod tests_wp_pghdev {
    use super::*;

    fn device(protocol: Option<(u8, u8)>, ext: Option<bool>) -> RegisteredDevice {
        RegisteredDevice {
            handle: 3,
            identity: DeviceIdentity {
                serial: "PGHDEV".into(),
                vid: 0x2E8B,
                location_id: 0,
                platform: 1,
                firmware: "1.1.7".into(),
                firmware_beta: None,
                outputs: 9,
                board_name: None,
                board: 0,
                protocol,
                ext,
            },
            name: "DSPi".into(),
            state: DeviceState::Online,
            locked_by: None,
        }
    }

    /// Spec 1.3 section 7.4: `protocol` for a paged device only; a legacy device is
    /// listed without it (OD-PGg). `ext` is ours, left out when unknown.
    #[test]
    fn wp_pghdev_the_device_object_says_protocol_and_ext() {
        let paged = device(Some((1, 0)), Some(true)).to_json();
        assert_eq!(paged["protocol"], json!({"major": 1, "minor": 0}));
        assert_eq!(paged["ext"], json!(true));
        let legacy = device(None, Some(false)).to_json();
        assert!(legacy.get("protocol").is_none(), "{legacy}");
        assert_eq!(legacy["ext"], json!(false));
        let unknown = device(None, None).to_json();
        assert!(unknown.get("ext").is_none(), "{unknown}");
        // The client's own parser reads it back.
        let info: dspi_link::messages::DeviceInfo = serde_json::from_value(paged).unwrap();
        assert!(info.is_paged());
        assert_eq!(info.ext, Some(true));
        let info: dspi_link::messages::DeviceInfo = serde_json::from_value(legacy).unwrap();
        assert!(!info.is_paged());
    }
}
