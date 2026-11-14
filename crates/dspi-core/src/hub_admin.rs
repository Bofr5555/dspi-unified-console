//! The hub's admin API over DSPi Link (WP HUBADMIN): an Installer page edits
//! the HUB's limits profile, installer PIN and pinned modes when the open
//! device is reached through a hub, and this console's own copy (P1LM, P1LK)
//! only when it is connected directly (USB, MIDI, the demo).
//!
//! - [`DspiCore::installer_target`] says which copy the page edits, whether
//!   this console may change it, and the notice row that says so. No I/O.
//! - [`DspiCore::hub_limits_get`], [`DspiCore::hub_limits_set`],
//!   [`DspiCore::hub_limits_clear`], [`DspiCore::hub_pin_set`] (a recovery
//!   code shown once, OD-P1c), [`DspiCore::hub_pin_clear`] and
//!   [`DspiCore::hub_mode_pin`]: typed calls over the hub's `limits.*` and
//!   `mode.*` messages (`docs/integrator/hub.md`). Each is a round trip: off
//!   the UI thread.
//! - The hub's `limits.changed`, `limits.held` and `mode.changed` events
//!   reach the core through `dspi-link`'s message hook: the core reads the
//!   lock again ([`DspiCore::installer_lock_refresh`]) and tells the app
//!   ([`HubAdminListener`], or [`DspiCore::hub_admin_events`] where a
//!   callback cannot reach the app, as on the web).
//!
//! The coordinator's rules hold as the hub keeps them: pairing with the hub's
//! PIN gives Control at most, so Admin needs `uconsole hub --pair-admin` (or
//! the role set on the hub's machine); wrong installer PINs are counted per
//! device on the hub (3 free, then 30 s doubling to 15 min, kept across a
//! restart); the analyser stays allowed while locked. Below Admin the page is
//! read-only, with [`NOTICE_HUB_ROLE`] saying why.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::{DspiCore, DspiError, LockState, Result, Slot};

/// The Installer page edits the hub's copy ([`InstallerTarget::copy`]).
pub const COPY_HUB: &str = "hub";
/// The Installer page edits this console's own copy (USB, MIDI, the demo).
pub const COPY_CONSOLE: &str = "console";
/// There is nothing to edit: no device, or a hub without `limits`.
pub const COPY_NONE: &str = "none";

/// Notice row: the page edits the hub's copy.
pub const NOTICE_HUB_COPY: &str = "installer.hub_copy";
/// Notice row: the page edits this console's copy (connected over USB).
pub const NOTICE_CONSOLE_COPY: &str = "installer.console_copy";
/// Notice row: read-only, this console's role on the hub is below Admin.
pub const NOTICE_HUB_ROLE: &str = "installer.hub_role";
/// Notice row (and the `dlink.no_limits` error's): the hub keeps no limits.
pub const NOTICE_NO_LIMITS: &str = "dlink.no_limits";
/// Notice row: the recovery code of a PIN set on the hub.
pub const NOTICE_HUB_RECOVERY: &str = "installer_lock.hub_recovery";

/// The roles of a hub, in the order a page lists them.
pub const HUB_ROLES: [&str; 3] = ["viewer", "control", "admin"];

/// The events kept for [`DspiCore::hub_admin_events`] at most; older ones go.
const QUEUE_MAX: usize = 32;
/// Rows of a hold kept per event.
const ROWS_MAX: usize = 64;
/// Longest text taken from the hub for one field.
const TEXT_MAX: usize = 300;

// ── Records ────────────────────────────────────────────────────────────────

/// Which copy of the limits, the installer PIN and the pinned modes the
/// Installer page edits ([`DspiCore::installer_target`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerTarget {
    /// [`COPY_HUB`], [`COPY_CONSOLE`] or [`COPY_NONE`].
    pub copy: String,
    /// The page may change that copy: always for this console's own (its PIN
    /// prompt is [`LockState::installer_open`]'s), for the hub's only as an
    /// Admin of the hub.
    pub editable: bool,
    /// The open device's serial ("" with none).
    pub serial: String,
    /// This console's role on the hub (`viewer`, `control`, `admin`): the
    /// one the hub last answered [`DspiCore::hub_limits_get`] with, else the
    /// session's; "" off a hub, or while the hub session is down.
    pub role: String,
    /// The notice row that says which copy is edited: [`NOTICE_HUB_COPY`],
    /// [`NOTICE_CONSOLE_COPY`], [`NOTICE_NO_LIMITS`], or `disconnected`.
    pub notice_key: String,
    /// Why the page is read-only, as a notice row ([`NOTICE_HUB_ROLE`],
    /// [`NOTICE_NO_LIMITS`], `disconnected`); "" when it is editable.
    pub read_only_key: String,
}

/// A role's pinned mode on the hub.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubModePin {
    /// `viewer`, `control` or `admin`.
    pub role: String,
    /// `simple`, `advanced` or `installer`; "" for the user's choice.
    pub mode: String,
}

/// The hub's limits, PIN and pinned modes for the open device
/// ([`DspiCore::hub_limits_get`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubAdminState {
    /// The device's serial.
    pub serial: String,
    /// This console's role on the hub.
    pub role: String,
    /// This console may change the hub's copy (it is an Admin of the hub).
    pub can_edit: bool,
    /// The hub's profile as a limits document (the `.dspipreset` family,
    /// `meta.kind` `limits`); "" when the hub has none for the device.
    pub profile_document: String,
    /// A PIN is set on the hub for the device.
    pub pin_set: bool,
    /// The hub's lock is on (a PIN set and the profile's `lockedPages`
    /// non-empty).
    pub locked: bool,
    /// The lock does not hold for this console (unlocked, or an Admin).
    pub unlocked: bool,
    /// What the hub last held the device to (its words).
    pub held: Vec<String>,
    /// Wrong PINs since the last good one (counted per device on the hub).
    pub wrong_tries: u32,
    /// The hub's refusal window left, ms; 0: none.
    pub retry_after_ms: u64,
    /// The pinned mode of every role (an Admin sees all three), or only this
    /// console's role's.
    pub pinned_modes: Vec<HubModePin>,
}

/// A profile set on the hub ([`DspiCore::hub_limits_set`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubLimitsSet {
    /// What the hub held the device to at once (its words), as `limits.held`.
    pub held: Vec<String>,
    /// The hub's state after the change.
    pub state: HubAdminState,
}

/// A PIN set on the hub ([`DspiCore::hub_pin_set`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubPinSet {
    /// The recovery code (`XXXXX-XXXXX-XXXXX-XXXXX`): unlocks the device on
    /// the hub in place of the PIN. Shown this once, never kept but as a
    /// hash on the hub (notice [`NOTICE_HUB_RECOVERY`]).
    pub recovery_code: String,
    /// The hub's state after the change.
    pub state: HubAdminState,
}

/// One of the hub's admin events, after the core read its lock again.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubAdminEvent {
    /// `limits.changed` (profile or PIN), `limits.held` (the hub held the
    /// device to its profile) or `mode.changed` (a role's pinned mode).
    pub kind: String,
    /// The device it concerns ("" for `mode.changed`, which is hub-wide).
    pub serial: String,
    /// `limits.held`'s rows.
    pub rows: Vec<String>,
    /// `mode.changed`'s role.
    pub role: String,
    /// `mode.changed`'s mode ("" unpinned).
    pub mode: String,
    /// The open device's lock after the refresh.
    pub lock: LockState,
}

/// The kinds of [`HubAdminEvent`].
const KINDS: [&str; 3] = ["limits.changed", "limits.held", "mode.changed"];

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str)
        .map(|t| t.chars().take(TEXT_MAX).collect())
        .unwrap_or_default()
}

fn rows(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(ROWS_MAX)
                .filter_map(Value::as_str)
                .map(|t| t.chars().take(TEXT_MAX).collect())
                .collect()
        })
        .unwrap_or_default()
}

impl HubAdminEvent {
    /// The event in a hub message of type `t`; `None` for any other message.
    pub(crate) fn from_hub(t: &str, body: &Value) -> Option<Self> {
        if !KINDS.contains(&t) {
            return None;
        }
        Some(Self {
            kind: t.to_owned(),
            serial: text(body.get("serial")),
            rows: rows(body.get("rows")),
            role: text(body.get("role")),
            mode: text(body.get("mode")),
            lock: LockState::default(),
        })
    }
}

/// The hub's admin events reach the app here (WP HUBADMIN): set with
/// [`DspiCore::set_hub_admin_listener`].
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait HubAdminListener: Send + Sync {
    /// The hub's limits, lock or pinned modes changed; `event.lock` is the
    /// open device's lock as read just now. Called on the Link session's
    /// supervisor, no core lock held: re-read the Installer page (off the UI
    /// thread) and return.
    fn on_hub_admin_event(&self, event: HubAdminEvent);
}

/// The listener and the queue of [`DspiCore::hub_admin_events`].
#[derive(Default)]
pub(crate) struct Events {
    listener: Slot<dyn HubAdminListener>,
    queue: Mutex<VecDeque<HubAdminEvent>>,
    /// The role the hub last answered for (serial, role): a role given on
    /// the hub's machine applies at once, while the session's hello keeps the
    /// role it paired with.
    role: Mutex<Option<(String, String)>>,
}

// ── The calls ──────────────────────────────────────────────────────────────

fn hub_error(e: DspiError) -> DspiError {
    // The hub's own refusals, as the core's kinds (`docs/integrator/hub.md`).
    match e {
        DspiError::Link { code, msg } => match code.as_str() {
            "denied" => DspiError::Denied,
            "bad_request" | "bad_pin" => DspiError::InvalidArgument { msg },
            "no_device" => DspiError::Disconnected,
            _ => DspiError::Link { code, msg },
        },
        other => other,
    }
}

fn no_limits() -> DspiError {
    DspiError::Link {
        code: "no_limits".into(),
        msg: "this DSPi is not reached through a hub that keeps limits".into(),
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Which copy of the limits, the installer PIN and the pinned modes the
    /// Installer page edits for the open device: the hub's when the device
    /// is reached through a hub that keeps them (editable by an Admin of the
    /// hub only), this console's own when it is connected directly (USB,
    /// MIDI, the demo), none otherwise. No I/O.
    pub fn installer_target(&self) -> InstallerTarget {
        let serial = self
            .lock()
            .conn
            .as_ref()
            .map(|c| c.serial.clone())
            .unwrap_or_default();
        if serial.is_empty() {
            return InstallerTarget {
                copy: COPY_NONE.into(),
                notice_key: "disconnected".into(),
                read_only_key: "disconnected".into(),
                ..InstallerTarget::default()
            };
        }
        #[cfg(feature = "link")]
        if let Some(t) = self.link_admin_target() {
            if !t.live {
                return InstallerTarget {
                    copy: COPY_HUB.into(),
                    serial,
                    notice_key: NOTICE_HUB_COPY.into(),
                    read_only_key: "disconnected".into(),
                    ..InstallerTarget::default()
                };
            }
            if !t.has_limits {
                return InstallerTarget {
                    copy: COPY_NONE.into(),
                    serial,
                    role: t.role,
                    notice_key: NOTICE_NO_LIMITS.into(),
                    read_only_key: NOTICE_NO_LIMITS.into(),
                    ..InstallerTarget::default()
                };
            }
            let role = self.hub_role_seen(&t.serial).unwrap_or(t.role);
            let editable = role == "admin";
            return InstallerTarget {
                copy: COPY_HUB.into(),
                editable,
                serial,
                role,
                notice_key: NOTICE_HUB_COPY.into(),
                read_only_key: if editable {
                    String::new()
                } else {
                    NOTICE_HUB_ROLE.into()
                },
            };
        }
        InstallerTarget {
            copy: COPY_CONSOLE.into(),
            editable: true,
            serial,
            role: String::new(),
            notice_key: NOTICE_CONSOLE_COPY.into(),
            read_only_key: String::new(),
        }
    }

    /// The hub's limits profile, PIN state and pinned modes for the open
    /// device (`limits.get`, `mode.pinned`). Every role may read them. Slow
    /// (two round trips): off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `no_limits` when the device is not
    /// reached through a hub that keeps limits (notice `dlink.no_limits`), or
    /// the hub's failure; [`DspiError::Disconnected`].
    pub fn hub_limits_get(&self) -> Result<HubAdminState> {
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            self.hub_state(handle)
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Set the hub's limits profile for the open device: `document` is a
    /// limits document (or one carrying `meta.limits`). The hub holds the
    /// device to it at once and says what it held. Admin of the hub only.
    /// The lock is read again. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a document that is not a
    /// limits profile; [`DspiError::Denied`] below Admin; as
    /// [`DspiCore::hub_limits_get`].
    pub fn hub_limits_set(&self, document: String) -> Result<HubLimitsSet> {
        let doc: Value =
            serde_json::from_str(&document).map_err(|e| DspiError::InvalidArgument {
                msg: format!("not a limits profile: {e}"),
            })?;
        dspi_session::limits::from_value(&doc)
            .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            let reply = self.hub_admin_send(handle, "limits.set", "profile", doc)?;
            let held = rows(reply.get("held"));
            let state = self.hub_after_change(handle)?;
            Ok(HubLimitsSet { held, state })
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Remove the hub's limits profile for the open device (nothing on the
    /// device changes; with it goes what the lock covers). Admin of the hub
    /// only. Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_limits_set`].
    pub fn hub_limits_clear(&self) -> Result<HubAdminState> {
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            self.hub_admin_send(handle, "limits.set", "profile", Value::Null)?;
            self.hub_after_change(handle)
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Set (or change) the hub's installer PIN for the open device: the hub
    /// keeps only its salted, slow hash, and makes a recovery code shown this
    /// once. Every console's unlock of the device ends. Admin of the hub only
    /// (an Admin needs no PIN). Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a PIN of fewer than 4 or
    /// more than 64 characters, or with a space at either end; as
    /// [`DspiCore::hub_limits_set`].
    pub fn hub_pin_set(&self, pin: String) -> Result<HubPinSet> {
        crate::limits::check_pin_text(&pin)?;
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            let reply = self.hub_admin_send(handle, "limits.pin", "pin", Value::from(pin))?;
            let recovery_code = text(reply.get("recovery_code"));
            let state = self.hub_after_change(handle)?;
            Ok(HubPinSet {
                recovery_code,
                state,
            })
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Remove the hub's installer PIN for the open device (and its recovery
    /// code): the hub's lock is off. Admin of the hub only. Slow: off the UI
    /// thread.
    ///
    /// Errors: as [`DspiCore::hub_limits_set`].
    pub fn hub_pin_clear(&self) -> Result<HubAdminState> {
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            self.hub_admin_send(handle, "limits.pin", "pin", Value::Null)?;
            self.hub_after_change(handle)
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Pin `role`'s mode on the hub (`viewer`, `control`, `admin`; `mode`
    /// one of the manifest's modes, "" to unpin). Hub-wide: every console of
    /// that role shows it. Admin of the hub only. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown role or mode; as
    /// [`DspiCore::hub_limits_set`].
    pub fn hub_mode_pin(&self, role: String, mode: String) -> Result<HubAdminState> {
        if !HUB_ROLES.contains(&role.as_str()) {
            return Err(DspiError::InvalidArgument {
                msg: format!("\"{role}\" is not a hub role (viewer, control, admin)"),
            });
        }
        if !mode.is_empty() && !crate::mode_manifest().modes.iter().any(|m| m.id == mode) {
            return Err(DspiError::InvalidArgument {
                msg: format!("\"{mode}\" is not a mode of the manifest"),
            });
        }
        #[cfg(feature = "link")]
        {
            let (handle, _) = self.hub_admin_target()?;
            let mut f = serde_json::Map::new();
            f.insert("role".into(), Value::from(role));
            f.insert(
                "mode".into(),
                if mode.is_empty() {
                    Value::Null
                } else {
                    Value::from(mode)
                },
            );
            self.link_hub_request("mode.pin", f).map_err(hub_error)?;
            self.hub_after_change(handle)
        }
        #[cfg(not(feature = "link"))]
        Err(no_limits())
    }

    /// Register (`Some`) or clear (`None`) the listener for the hub's admin
    /// events ([`HubAdminListener`]).
    pub fn set_hub_admin_listener(&self, listener: Option<Arc<dyn HubAdminListener>>) {
        self.listeners.hub_admin.listener.set(listener);
    }

    /// The hub's admin events since the last call, oldest first (at most
    /// 32 are kept), for a binding that polls instead of a listener (the
    /// web). No I/O.
    pub fn hub_admin_events(&self) -> Vec<HubAdminEvent> {
        self.listeners
            .hub_admin
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl DspiCore {
    /// The role the hub last answered with for `serial`, if any.
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    fn hub_role_seen(&self, serial: &str) -> Option<String> {
        self.listeners
            .hub_admin
            .role
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(s, r)| s == serial && !r.is_empty())
            .map(|(_, r)| r.clone())
    }

    /// An event of the hub's: the lock read again, then kept and told. On
    /// the Link supervisor, with no core lock held.
    pub(crate) fn hub_admin_event(&self, mut event: HubAdminEvent) {
        #[cfg(feature = "link")]
        {
            // Another device's limits: not this console's business.
            if event.kind != "mode.changed" {
                let ours = self
                    .link_admin_target()
                    .is_some_and(|t| t.serial == event.serial);
                if !ours {
                    return;
                }
            }
            event.lock = self
                .installer_lock_refresh()
                .unwrap_or_else(|_| self.installer_lock_state());
        }
        #[cfg(not(feature = "link"))]
        {
            event.lock = self.installer_lock_state();
        }
        {
            let mut q = self
                .listeners
                .hub_admin
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if q.len() >= QUEUE_MAX {
                q.pop_front();
            }
            q.push_back(event.clone());
        }
        self.listeners
            .hub_admin
            .listener
            .call(|l| l.on_hub_admin_event(event));
    }
}

#[cfg(feature = "link")]
impl DspiCore {
    /// The open device's handle and serial on a live hub that keeps limits.
    /// Whether this console may write is the hub's to judge (Admin only):
    /// the role it knows is the live one.
    fn hub_admin_target(&self) -> Result<(u8, String)> {
        if self.lock().conn.is_none() {
            return Err(DspiError::Disconnected);
        }
        let t = self.link_admin_target().ok_or_else(no_limits)?;
        if !t.live {
            return Err(DspiError::Disconnected);
        }
        if !t.has_limits {
            return Err(no_limits());
        }
        Ok((t.handle, t.serial))
    }

    /// Send `t` for `handle` with `key: value`; the hub's `ok` whole.
    fn hub_admin_send(&self, handle: u8, t: &str, key: &str, value: Value) -> Result<Value> {
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), handle.into());
        f.insert(key.into(), value);
        self.link_hub_request(t, f).map_err(hub_error)
    }

    /// After a change: the core's lock read again (the hub said
    /// `limits.changed` too, but this caller should not wait for it), then
    /// the hub's state.
    fn hub_after_change(&self, handle: u8) -> Result<HubAdminState> {
        let _ = self.installer_lock_refresh();
        self.hub_state(handle)
    }

    /// `limits.get` and `mode.pinned` for `handle`, as one record.
    fn hub_state(&self, handle: u8) -> Result<HubAdminState> {
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), handle.into());
        let got = self
            .link_hub_request("limits.get", f.clone())
            .map_err(hub_error)?;
        let pinned = self.link_hub_request("mode.pinned", f).map_err(hub_error)?;
        let role = text(pinned.get("role"));
        let st = state_from(&got, &pinned, role);
        *self
            .listeners
            .hub_admin
            .role
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((st.serial.clone(), st.role.clone()));
        Ok(st)
    }
}

/// The record from the hub's `limits.get` and `mode.pinned` answers.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn state_from(got: &Value, pinned: &Value, role: String) -> HubAdminState {
    let flag = |k: &str| got.get(k).and_then(Value::as_bool).unwrap_or(false);
    let profile_document = got
        .get("profile")
        .filter(|p| p.is_object())
        .and_then(|p| {
            let doc = serde_json::json!({ "schemaVersion": 1, "meta": { "kind": "limits" }, "limits": p });
            dspi_session::limits::from_value(&doc).ok()
        })
        .map(|p| p.to_document())
        .unwrap_or_default();
    let mode_of = |v: Option<&Value>| text(v);
    let pinned_modes = match pinned.get("pins").and_then(Value::as_object) {
        Some(pins) => HUB_ROLES
            .iter()
            .map(|r| HubModePin {
                role: (*r).to_owned(),
                mode: mode_of(pins.get(*r)),
            })
            .collect(),
        None if !role.is_empty() => vec![HubModePin {
            role: role.clone(),
            mode: mode_of(pinned.get("mode")),
        }],
        None => Vec::new(),
    };
    HubAdminState {
        serial: text(got.get("serial")),
        can_edit: role == "admin",
        role,
        profile_document,
        pin_set: flag("pin_set"),
        locked: flag("locked"),
        unlocked: flag("unlocked"),
        held: rows(got.get("held")),
        wrong_tries: got
            .get("wrong_tries")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        retry_after_ms: got.get("retry_in_ms").and_then(Value::as_u64).unwrap_or(0),
        pinned_modes,
    }
}

#[cfg(test)]
#[path = "hub_admin/tests_wp_hubadmin.rs"]
mod tests_wp_hubadmin;
