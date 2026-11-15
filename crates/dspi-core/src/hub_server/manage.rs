//! The hub's device and name writes (WP P2CC3, the third section of
//! `hub_server`): an Admin's inventory edits, forgetting a device, renaming
//! the hub, and Identify for any device on the hub. The wire contract is
//! `docs/integrator/hub-api.md` sections 3, 4 and 15; each call's name
//! follows its message.
//!
//! - [`DspiCore::hub_inventory_set`] (`inventory.set`, Admin): a device's
//!   location, note and tags.
//! - [`DspiCore::hub_inventory_forget`] (`inventory.forget`, Admin): an
//!   offline device's record, its history and, if asked, its snapshots; the
//!   hub refuses it while the device is online (`conflict`).
//! - [`DspiCore::hub_rename`] (`hub.rename`, Admin): the hub's name.
//! - [`DspiCore::hub_identify_serial`] (`device.identify`, Control or
//!   Admin): any device on the hub, not only the open one
//!   ([`DspiCore::hub_identify`]).
//!
//! As the first section: [`DspiError::Unsupported`] with nothing sent
//! without the family's cap (or on no hub), the hub's refusals typed, and
//! the client read-only mode refuses all four with nothing sent (none is in
//! [`super::HUB_SERVER_READS`]; Identify writes to the device).

use serde::Serialize;
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use serde_json::{Map, Value};

#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use super::{CAP_HUB_STATS, CAP_IDENTIFY, CAP_INVENTORY, HubDevice, device_from, text, texts};
use crate::{DspiCore, DspiError, IdentifyInfo, Result};

/// What [`DspiCore::hub_inventory_forget`] removed.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInventoryForgot {
    /// The device.
    pub serial: String,
    /// What went, in order: `record`, then `events` if it had history files,
    /// then `snapshots` if asked and it had any.
    pub forgot: Vec<String>,
}

#[cfg(feature = "link")]
fn protocol(msg: &str) -> DspiError {
    DspiError::Link {
        code: "protocol".into(),
        msg: msg.into(),
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Set device `serial`'s location, note and tags in the hub's inventory
    /// ("" the open device; `inventory.set`). Each `Some` replaces its field
    /// and an empty one clears it (`Some("")`, `Some(vec![])`); `None` keeps
    /// it. The hub's bounds: a location and a note at most 256 characters, no
    /// control characters (a note may hold newlines and tabs); at most 16
    /// tags of 1 to 32 characters, trimmed, duplicates dropped. The answer is
    /// the record as the hub now has it. Admin of the hub. Slow: off the UI
    /// thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `inventory` cap (nothing
    /// sent); [`DspiError::InvalidArgument`] for a value past a bound (the
    /// hub's `bad_request`, nothing changed); [`DspiError::Link`] `not_found`
    /// for a serial not in the inventory; [`DspiError::Denied`] below Admin;
    /// [`DspiError::ReadOnly`] (nothing sent); [`DspiError::Disconnected`].
    pub fn hub_inventory_set(
        &self,
        serial: String,
        location: Option<String>,
        note: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<HubDevice> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_INVENTORY)?;
            let serial = self.hub_serial(serial)?;
            let mut f = Map::new();
            f.insert("serial".into(), serial.into());
            if let Some(l) = location {
                f.insert("location".into(), l.into());
            }
            if let Some(n) = note {
                f.insert("note".into(), n.into());
            }
            if let Some(t) = tags {
                f.insert("tags".into(), t.into());
            }
            let reply = self.hub_ask("inventory.set", f)?;
            reply
                .get("device")
                .filter(|d| d.is_object())
                .map(device_from)
                .ok_or_else(|| protocol("inventory.set's answer has no device"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (serial, location, note, tags);
            Err(DspiError::Unsupported)
        }
    }

    /// Forget device `serial` on the hub (`inventory.forget`): its record and
    /// its history files, and with `with_data` its snapshots on the hub too.
    /// Only an offline device: the hub refuses one that is attached. Admin of
    /// the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an empty `serial` (nothing
    /// sent); [`DspiError::Link`] `conflict` while the device is online,
    /// `not_found` for a serial not in the inventory, `internal` when a file
    /// could not be removed (the record stays: try again); as
    /// [`DspiCore::hub_inventory_set`].
    pub fn hub_inventory_forget(
        &self,
        serial: String,
        with_data: bool,
    ) -> Result<HubInventoryForgot> {
        if serial.is_empty() {
            return Err(DspiError::InvalidArgument {
                msg: "forget needs the device's serial".into(),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_INVENTORY)?;
            let mut f = Map::new();
            f.insert("serial".into(), serial.clone().into());
            f.insert("with_data".into(), with_data.into());
            let reply = self.hub_ask("inventory.forget", f)?;
            let answered = text(reply.get("serial"));
            Ok(HubInventoryForgot {
                serial: if answered.is_empty() {
                    serial
                } else {
                    answered
                },
                forgot: texts(reply.get("forgot")),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = with_data;
            Err(DspiError::Unsupported)
        }
    }

    /// Rename the hub (`hub.rename`): control characters dropped and the ends
    /// trimmed by the hub, at most 64 characters. The answer is the name the
    /// hub now uses (in every later `hello`, `/dspi/v1/info` and its mDNS
    /// record). Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `hub_stats` cap
    /// (nothing sent); [`DspiError::InvalidArgument`] for a name empty after
    /// that or too long (nothing changed); [`DspiError::Denied`] below Admin;
    /// [`DspiError::ReadOnly`] (nothing sent); [`DspiError::Disconnected`].
    pub fn hub_rename(&self, name: String) -> Result<String> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_STATS)?;
            let mut f = Map::new();
            f.insert("name".into(), name.into());
            let reply = self.hub_ask("hub.rename", f)?;
            if !reply.get("name").is_some_and(Value::is_string) {
                return Err(protocol("hub.rename's answer has no name"));
            }
            Ok(text(reply.get("name")))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = name;
            Err(DspiError::Unsupported)
        }
    }

    /// Identify device `serial` on the hub ("" the open device;
    /// `device.identify`): blink its LED for `seconds` (0 stops it; at most
    /// 120, longer is shortened) and, with `tone_output`, play its quiet
    /// identify tone once on that output (leave it empty unless the user
    /// asked for sound). Any device on the hub, open here or not; the hub
    /// sends it, at most once per device per 2 s, whoever asked. The answer
    /// is the device's state read after the write; `None` when that read
    /// failed (the write went out). Control or Admin of the hub. Slow: off
    /// the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] (nothing sent) on no hub, on a hub
    /// without the `identify` cap, and on firmware without `EXT_IDENTIFY`;
    /// [`DspiError::InvalidArgument`] for a tone output the device does not
    /// have; [`DspiError::Locked`] for a tone under another session's lock;
    /// [`DspiError::Link`] `rate_limited`, `offline`, `not_found`, `device`;
    /// [`DspiError::Denied`] for a Viewer; [`DspiError::ReadOnly`] (nothing
    /// sent); [`DspiError::Disconnected`].
    pub fn hub_identify_serial(
        &self,
        serial: String,
        seconds: u8,
        tone_output: Option<u8>,
    ) -> Result<Option<IdentifyInfo>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_IDENTIFY)?;
            let serial = self.hub_serial(serial)?;
            self.hub_identify_on(serial, seconds, tone_output)
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (serial, seconds, tone_output);
            Err(DspiError::Unsupported)
        }
    }
}

#[cfg(test)]
#[path = "tests_wp_p2cc3.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2cc3;
