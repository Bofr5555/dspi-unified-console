//! Devices by handle and serial (spec 7.4). Owned by WP HB3.
//!
//! Pure: the caller passes the clock, and every change returns its event so the
//! caller publishes it with the hub's lock released. HB0 has handles, states, the
//! offline grace and the `device.*` messages. HB3 adds names and their store, and the
//! board name's adoption.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::session::SessionId;

/// How long a device that dropped keeps its handle (Swift hub; spec 8.2 asks for
/// 60 s after a bootloader entry).
pub const OFFLINE_GRACE: Duration = Duration::from_secs(60);
/// Handles are 0..=254; 255 is reserved (spec 2).
const MAX_HANDLE: u8 = 254;

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
    /// unnamed. HB3 reads it.
    pub board_name: Option<String>,
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
}

impl Registry {
    /// An empty registry with this offline grace.
    pub fn new(grace: Duration) -> Self {
        Self {
            grace,
            by_serial: BTreeMap::new(),
        }
    }

    /// The device is present. `None` when no handle is free.
    pub fn online(&mut self, identity: DeviceIdentity) -> Option<Online> {
        if let Some(entry) = self.by_serial.get_mut(&identity.serial) {
            entry.offline_since = None;
            entry.device.identity = identity;
            entry.device.state = DeviceState::Online;
            return Some(Online {
                handle: entry.device.handle,
                returning: true,
                event: RegistryEvent::Changed(entry.device.clone()),
            });
        }
        let handle = self.free_handle()?;
        let device = RegisteredDevice {
            handle,
            name: default_name(&identity.serial),
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

    /// `device.rename` (spec 7.4).
    pub fn rename(&mut self, handle: u8, name: &str) -> Option<RegistryEvent> {
        let _ = (handle, name);
        todo!("WP HB3")
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
