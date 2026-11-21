//! WP INST0 (MPLAN 1.4): setting up the installation the hub keeps, the
//! audio link's site (`dspi_session::system_model`): adding a unit the hub
//! sees, giving it a room, a role and what feeds it, making rooms, zones and
//! groups, senders and fan-outs. Every client is a thin form over these calls;
//! the logic is here, once.
//!
//! - [`DspiCore::installation_view`] reads the model (`system.get`) and the
//!   units the hub sees that the site does not hold yet (`inventory.list`);
//!   it hands back a draft of that model.
//! - [`DspiCore::installation_draft_edit`] makes edits on a draft. Pure: it
//!   sends nothing, and a draft is only text the caller keeps (a client's
//!   form, an agent's server-side draft), so two editors never share one.
//! - [`DspiCore::installation_validate`] runs the hub's own validator on the
//!   draft; [`DspiCore::installation_diff`] says what saving would change.
//! - [`DspiCore::installation_save`] sends the draft as one `system.set`,
//!   guarded by the revision it was read at (`conflict` when the hub's model
//!   moved meanwhile), through the same read-modify-write as the "Clock and
//!   audio links" edits ([`DspiCore::alink_system_save_report`]). Admin only.
//!
//! The Terminal's `usb-direct generate` builds its model with the same edits
//! ([`installation_apply`]), so there is one path to a model.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use dspi_session::system_model::{
    FanOut, Group, Kind, MAX_TEXT, OnLockError, Room, Sender, Speaker, SystemModel, USB_MODES,
    Unit, UnitTransport, Zone,
};

use super::alink_system::AlinkSystemSaved;
use crate::{DspiCore, DspiError, Result};

/// A model being edited: the one read at `revision` (`base`) and the edits
/// made on it (`model`), both the hub's JSON as text. Saving sends `model`
/// with `revision`; the diff compares it with `base`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationDraft {
    /// The hub's revision the draft was read at (0 for a new installation).
    pub revision: u64,
    /// The model as read, JSON.
    pub base: String,
    /// The model with the edits, JSON.
    pub model: String,
}

/// A DSPi the hub sees that the installation does not hold.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationUnseen {
    /// Its USB serial: what [`InstallationEdit::AddUnit`] takes.
    pub serial: String,
    /// Its name when last seen.
    pub name: String,
    /// Attached now.
    pub online: bool,
    /// The firmware version; "" when unknown.
    pub fw: String,
    /// Output channels.
    pub outputs: u8,
}

/// What [`DspiCore::installation_view`] reads.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationView {
    /// The model's revision on the hub.
    pub revision: u64,
    /// This session's role on the hub (`viewer`, `control`, `admin`).
    pub role: String,
    /// The role may save (admin).
    pub may_edit: bool,
    /// A draft of the model as it is.
    pub draft: InstallationDraft,
    /// The units the hub sees that the installation does not hold, as the
    /// hub's inventory orders them (online first).
    pub unseen: Vec<InstallationUnseen>,
    /// The hub keeps an inventory: false for one without (`unseen` is then
    /// empty, not "none").
    pub inventory: bool,
}

/// A unit's role in the installation: how the programme reaches it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationRole {
    /// It plays the audio link from its upstream (a sender, a fan-out or a
    /// repeater).
    Receiver,
    /// A receiver that also re-emits the stream to the units naming it.
    Repeater,
    /// It follows an S/PDIF source unit's time base (its upstream).
    SpdifFollower,
    /// USB directly from the host; no upstream.
    UsbDirect,
}

/// A speaker on one of a unit's outputs.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationSpeaker {
    /// The unit's output, from 0.
    pub output: u8,
    /// From the speaker to the listening position, metres.
    pub distance_m: f32,
}

/// One edit of a draft. An `id` left out is made from the name (or the
/// serial); every id an edit names must exist, every serial is in the
/// installation once. JSON: `{"op": "addUnit", "serial": ..., ...}`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InstallationEdit {
    /// Add the unit with this serial, in `room`, as `role`, fed by
    /// `upstream` (none for a USB-direct unit).
    AddUnit {
        /// Its USB serial.
        serial: String,
        /// Its id; `None`: made from the name or the serial.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
        /// Where it stands.
        #[serde(default)]
        room: Option<String>,
        /// How the programme reaches it.
        role: InstallationRole,
        /// What feeds it: a sender, a fan-out or a repeater (an S/PDIF
        /// source for a follower).
        #[serde(default)]
        upstream: Option<String>,
    },
    /// Remove an element of any kind. What names it as a member (a zone's
    /// rooms, a group's units, a unit's room, a zone's source, a source's
    /// sender) forgets it, and scene actions aimed at it go; an element it
    /// fed is left for the validator to point out.
    RemoveElement {
        /// The element.
        id: String,
    },
    /// Rename an element of any kind.
    RenameElement {
        /// The element.
        id: String,
        /// The new name.
        name: String,
    },
    /// Put a unit in a room, or in none.
    SetUnitRoom {
        /// The unit.
        id: String,
        /// The room; `None`: no room.
        #[serde(default)]
        room: Option<String>,
    },
    /// Change a unit's role. The fields the new role cannot have go (a
    /// USB-direct unit's upstream, an S/PDIF follower's slots).
    SetUnitRole {
        /// The unit.
        id: String,
        /// Its new role.
        role: InstallationRole,
    },
    /// Set what feeds a unit, and which output of it.
    SetUnitUpstream {
        /// The unit.
        id: String,
        /// The element feeding it.
        upstream: String,
        /// Which output of the upstream element, for the record.
        #[serde(default)]
        output: Option<u8>,
    },
    /// A USB-direct unit's mode and what it does in lock error.
    SetUnitUsbMode {
        /// The unit.
        id: String,
        /// `off`, `rate_lock` or `phase_lock`; `None`: the unit's own.
        #[serde(default)]
        mode: Option<String>,
        /// Play unaligned in lock error instead of muting.
        #[serde(default)]
        play_unaligned: bool,
    },
    /// A unit's speakers by output. A speaker on an output that stays keeps
    /// its measured arrival.
    SetUnitSpeakers {
        /// The unit.
        id: String,
        /// Its speakers.
        speakers: Vec<InstallationSpeaker>,
    },
    /// Make a unit an S/PDIF time-base source on the output slots of
    /// `tx_mask`, or (0) not one.
    SetSpdifSource {
        /// The unit.
        id: String,
        /// Bit n: S/PDIF output slot n carries the time base; 0: not a source.
        tx_mask: u8,
    },
    /// Add a room.
    AddRoom {
        /// Its id; `None`: made from the name.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
    },
    /// Add a zone of `rooms`, playing `source`.
    AddZone {
        /// Its id; `None`: made from the name.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
        /// Its rooms.
        #[serde(default)]
        rooms: Vec<String>,
        /// What it plays.
        #[serde(default)]
        source: Option<String>,
    },
    /// A zone's rooms.
    SetZoneRooms {
        /// The zone.
        id: String,
        /// Its rooms.
        rooms: Vec<String>,
    },
    /// Add a group of `units`.
    AddGroup {
        /// Its id; `None`: made from the name.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
        /// Its units.
        #[serde(default)]
        units: Vec<String>,
    },
    /// A group's units.
    SetGroupUnits {
        /// The group.
        id: String,
        /// Its units.
        units: Vec<String>,
    },
    /// Add a sender (the audio link's clock).
    AddSender {
        /// Its id; `None`: made from the name or the serial.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
        /// Its USB serial when the hub can reach it.
        #[serde(default)]
        serial: Option<String>,
    },
    /// Add a fan-out fed by `upstream`.
    AddFanOut {
        /// Its id; `None`: made from the name or the serial.
        #[serde(default)]
        id: Option<String>,
        /// What people call it.
        #[serde(default)]
        name: String,
        /// Its USB serial when the hub can reach it.
        #[serde(default)]
        serial: Option<String>,
        /// What feeds it: a sender, a fan-out or a repeater.
        upstream: String,
        /// Which output of the upstream element, for the record.
        #[serde(default)]
        output: Option<u8>,
    },
    /// The installation's own settings; `None` leaves one as it is.
    SetSystem {
        /// 44100, 48000 or 96000.
        #[serde(default)]
        rate_hz: Option<u32>,
        /// Change the system delay to `d_frames`.
        #[serde(default)]
        set_d: bool,
        /// The system delay in frames; `None`: automatic.
        #[serde(default)]
        d_frames: Option<u16>,
        /// Keep USB-direct units aligned.
        #[serde(default)]
        keep_usb_aligned: Option<bool>,
    },
}

/// One reason the hub would refuse a draft.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationIssue {
    /// The element it is about; "" for the installation as a whole.
    pub subject: String,
    /// The hub's words.
    pub message: String,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The installation on the hub this session is on, as a draft to edit,
    /// with the units the hub sees that it does not hold. Every role may
    /// read it; `may_edit` says whether this one may save. Blocks on the
    /// network: keep it off a UI thread.
    ///
    /// Errors: [`DspiError::Disconnected`] with no hub session,
    /// [`DspiError::Link`] with the hub's code or the client's.
    pub fn installation_view(&self) -> Result<InstallationView> {
        let got = self.link_hub_request("system.get", Map::new())?;
        let revision = got.get("revision").and_then(Value::as_u64).unwrap_or(0);
        let model = match got.get("model") {
            None | Some(Value::Null) => SystemModel::default(),
            Some(m) => serde_json::from_value(m.clone()).map_err(|e| DspiError::Link {
                code: "protocol".into(),
                msg: format!("the hub's installation could not be read: {e}"),
            })?,
        };
        let role = got
            .get("role")
            .and_then(Value::as_str)
            .map_or_else(|| self.link_hub_role(), str::to_owned);
        let (unseen, inventory) = match self.hub_inventory() {
            Ok(devices) => {
                let held = serials(&model);
                let unseen = devices
                    .into_iter()
                    .filter(|d| !held.contains(d.serial.as_str()))
                    .map(|d| InstallationUnseen {
                        serial: d.serial,
                        name: d.name,
                        online: d.online,
                        fw: d.fw,
                        outputs: d.outputs,
                    })
                    .collect();
                (unseen, true)
            }
            Err(DspiError::Unsupported) => (Vec::new(), false),
            Err(e) => return Err(e),
        };
        Ok(InstallationView {
            revision,
            may_edit: role == "admin",
            role,
            draft: InstallationDraft::new(revision, &model)?,
            unseen,
            inventory,
        })
    }

    /// `edits` made on `draft`, in order, all or none. Sends nothing.
    ///
    /// Errors: [`DspiError::InvalidArgument`] naming the edit that cannot be
    /// made (an id that does not exist or is taken, a serial already in the
    /// installation, a name too long) or a draft that is not a model.
    pub fn installation_draft_edit(
        &self,
        draft: InstallationDraft,
        edits: Vec<InstallationEdit>,
    ) -> Result<InstallationDraft> {
        let mut model = draft.model_parsed()?;
        for (i, edit) in edits.iter().enumerate() {
            installation_apply(&mut model, edit).map_err(|e| match e {
                DspiError::InvalidArgument { msg } => DspiError::InvalidArgument {
                    msg: format!("edit {}: {msg}", i.saturating_add(1)),
                },
                other => other,
            })?;
        }
        Ok(InstallationDraft {
            model: to_text(&model)?,
            ..draft
        })
    }

    /// The hub's own validator on `draft`: empty when the hub would take it,
    /// else why not, naming the element. Sends nothing.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a draft that is not a model.
    pub fn installation_validate(
        &self,
        draft: InstallationDraft,
    ) -> Result<Vec<InstallationIssue>> {
        Ok(issues(&draft.model_parsed()?))
    }

    /// What saving `draft` would change, one sentence per change, in the
    /// model's order. Sends nothing.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a draft that is not a model.
    pub fn installation_diff(&self, draft: InstallationDraft) -> Result<Vec<String>> {
        Ok(installation_changes(
            &draft.base_parsed()?,
            &draft.model_parsed()?,
        ))
    }

    /// Save `draft` as the installation: one `system.set` the hub takes only
    /// while its model is still at the draft's revision. Checked here first
    /// with the hub's validator. Returns the new revision and the hub's
    /// warnings. Admin only. Blocks.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a draft the hub would
    /// refuse (nothing sent); [`DspiError::ReadOnly`] in the client
    /// read-only mode (nothing sent, not even the read); [`DspiError::Link`]
    /// `conflict` when the installation changed on the hub meanwhile (read
    /// it again), `denied` for another role; [`DspiError::Disconnected`].
    pub fn installation_save(&self, draft: InstallationDraft) -> Result<AlinkSystemSaved> {
        let model = draft.model_parsed()?;
        model
            .validate()
            .map_err(|e| DspiError::InvalidArgument { msg: e.0 })?;
        let value = serde_json::to_value(&model).map_err(encode_error)?;
        self.alink_system_save_with(draft.revision, move |m| {
            *m = value;
            Ok(())
        })
    }
}

impl InstallationDraft {
    /// A draft of `model` as read at `revision`.
    pub fn new(revision: u64, model: &SystemModel) -> Result<Self> {
        let text = to_text(model)?;
        Ok(Self {
            revision,
            base: text.clone(),
            model: text,
        })
    }

    /// A draft of an empty installation (revision 0): what the Terminal's
    /// generator starts from.
    pub fn empty() -> Self {
        // An empty model always encodes; the fallback is never used.
        Self::new(0, &SystemModel::default()).unwrap_or(Self {
            revision: 0,
            base: "{}".into(),
            model: "{}".into(),
        })
    }

    /// The edited model.
    pub fn model_parsed(&self) -> Result<SystemModel> {
        parse(&self.model, "model")
    }

    /// The model as read.
    pub fn base_parsed(&self) -> Result<SystemModel> {
        parse(&self.base, "base")
    }

    /// The draft as one JSON object, its models as objects rather than text
    /// (the C ABI's and the web's shape): `{revision, base, model}`.
    pub fn to_json(&self) -> Value {
        let obj = |t: &str| serde_json::from_str(t).unwrap_or(Value::Null);
        serde_json::json!({
            "revision": self.revision,
            "base": obj(&self.base),
            "model": obj(&self.model),
        })
    }

    /// [`Self::to_json`]'s shape back, each model checked to be one.
    pub fn from_json(v: &Value) -> Result<Self> {
        let model = |key: &str| -> Result<String> {
            let m: SystemModel = serde_json::from_value(v.get(key).cloned().unwrap_or(Value::Null))
                .map_err(|e| DspiError::InvalidArgument {
                    msg: format!("the draft's {key}: {e}"),
                })?;
            to_text(&m)
        };
        Ok(Self {
            revision: v.get("revision").and_then(Value::as_u64).unwrap_or(0),
            base: model("base")?,
            model: model("model")?,
        })
    }
}

fn parse(text: &str, what: &str) -> Result<SystemModel> {
    serde_json::from_str(text).map_err(|e| DspiError::InvalidArgument {
        msg: format!("the draft's {what}: {e}"),
    })
}

fn encode_error(e: serde_json::Error) -> DspiError {
    DspiError::Io {
        msg: format!("the installation could not be encoded: {e}"),
    }
}

fn to_text(model: &SystemModel) -> Result<String> {
    serde_json::to_string(model).map_err(encode_error)
}

/// The hub's validator's verdict as issues (it stops at the first).
pub(crate) fn issues(model: &SystemModel) -> Vec<InstallationIssue> {
    match model.validate() {
        Ok(()) => Vec::new(),
        Err(e) => vec![InstallationIssue {
            subject: e.subject().unwrap_or_default(),
            message: e.0,
        }],
    }
}

/// Every serial the installation holds: its units', senders' and fan-outs'.
fn serials(m: &SystemModel) -> BTreeSet<&str> {
    m.units
        .iter()
        .map(|u| u.serial.as_str())
        .chain(m.senders.iter().filter_map(|s| s.serial.as_deref()))
        .chain(m.fan_outs.iter().filter_map(|f| f.serial.as_deref()))
        .collect()
}

// ------------------------------------------------------------------ edits

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(DspiError::InvalidArgument { msg: msg.into() })
}

fn text_ok(what: &str, t: &str) -> Result<()> {
    if t.chars().count() > MAX_TEXT || t.chars().any(char::is_control) {
        return invalid(format!(
            "{what}: at most {MAX_TEXT} characters, none of them control characters"
        ));
    }
    Ok(())
}

fn id_ok(id: &str) -> Result<()> {
    if id.is_empty() || id.contains(':') || id == "all" {
        return invalid(format!("id {id:?}: not empty, no ':', not \"all\""));
    }
    text_ok("an id", id)
}

fn serial_ok(m: &SystemModel, serial: &str) -> Result<()> {
    if serial.is_empty() {
        return invalid("a serial is empty");
    }
    text_ok("a serial", serial)?;
    if serials(m).contains(serial) {
        return invalid(format!("serial {serial:?} is in the installation already"));
    }
    Ok(())
}

/// `wanted`, checked and free; or an id made from `from` (else `kind`),
/// with `-2`, `-3`, ... while it is taken.
fn new_id(m: &SystemModel, wanted: Option<&str>, from: &str, kind: &str) -> Result<String> {
    let kinds = m.kinds();
    if let Some(w) = wanted {
        id_ok(w)?;
        if kinds.contains_key(w) {
            return invalid(format!("id {w:?} is taken"));
        }
        return Ok(w.to_owned());
    }
    let base: String = from
        .chars()
        .filter(|c| !c.is_control() && *c != ':')
        .take(MAX_TEXT.saturating_sub(4))
        .collect();
    let base = match base.trim() {
        "" | "all" => kind.to_owned(),
        b => b.to_owned(),
    };
    if !kinds.contains_key(base.as_str()) {
        return Ok(base);
    }
    (2..=1000u32)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !kinds.contains_key(c.as_str()))
        .map_or_else(|| invalid(format!("no free id for {base:?}")), Ok)
}

fn need(m: &SystemModel, id: &str, kind: Kind, what: &str) -> Result<()> {
    if m.kinds().get(id) == Some(&kind) {
        Ok(())
    } else {
        invalid(format!("no {what} {id:?}"))
    }
}

fn need_all(m: &SystemModel, ids: &[String], kind: Kind, what: &str) -> Result<()> {
    ids.iter().try_for_each(|id| need(m, id, kind, what))
}

fn unit_mut<'a>(m: &'a mut SystemModel, id: &str) -> Result<&'a mut Unit> {
    match m.units.iter_mut().find(|u| u.id == id) {
        Some(u) => Ok(u),
        None => invalid(format!("no unit {id:?}")),
    }
}

/// `role` on `u`, dropping what that role cannot have.
fn set_role(u: &mut Unit, role: InstallationRole) {
    let was = u.transport;
    match role {
        InstallationRole::Receiver | InstallationRole::Repeater => {
            u.transport = UnitTransport::Alink;
            u.repeater = role == InstallationRole::Repeater;
            u.usb_mode = None;
            u.on_lock_error = OnLockError::Mute;
        }
        InstallationRole::SpdifFollower => {
            u.transport = UnitTransport::Spdif;
            u.repeater = false;
            u.slots = None;
            u.hop_delay_frames = None;
            u.usb_mode = None;
            u.spdif_source = false;
            u.spdif_tx_mask = 0;
            u.spdif_d = None;
        }
        InstallationRole::UsbDirect => {
            u.transport = UnitTransport::UsbDirect;
            u.upstream.clear();
            u.output = None;
            u.repeater = false;
            u.slots = None;
            u.hop_delay_frames = None;
        }
    }
    // An upstream of another transport feeds nothing on this one.
    if was != u.transport {
        u.upstream.clear();
        u.output = None;
    }
}

/// `edit` made on `m`. Checks what the edit names, not the whole model
/// ([`SystemModel::validate`] does that). For Rust callers that build a
/// model (the Terminal's generator); the bindings use
/// [`DspiCore::installation_draft_edit`].
///
/// Errors: [`DspiError::InvalidArgument`] for an edit that cannot be made.
pub fn installation_apply(m: &mut SystemModel, edit: &InstallationEdit) -> Result<()> {
    use InstallationEdit as E;
    match edit {
        E::AddUnit {
            serial,
            id,
            name,
            room,
            role,
            upstream,
        } => {
            serial_ok(m, serial)?;
            text_ok("a name", name)?;
            if let Some(r) = room {
                need(m, r, Kind::Room, "room")?;
            }
            let upstream = upstream.clone().unwrap_or_default();
            if !upstream.is_empty() {
                if *role == InstallationRole::UsbDirect {
                    return invalid("a USB-direct unit has no upstream");
                }
                if !m.kinds().contains_key(upstream.as_str()) {
                    return invalid(format!("no element {upstream:?} to feed it"));
                }
            }
            let from = if name.is_empty() { serial } else { name };
            let id = new_id(m, id.as_deref(), from, "unit")?;
            let mut u = Unit {
                id,
                name: name.clone(),
                serial: serial.clone(),
                room: room.clone(),
                upstream: String::new(),
                transport: UnitTransport::Alink,
                usb_mode: None,
                on_lock_error: OnLockError::Mute,
                output: None,
                repeater: false,
                hop_delay_frames: None,
                slots: None,
                latency_us: 0,
                measured_latency_us: None,
                speakers: Vec::new(),
                spdif_source: false,
                spdif_tx_mask: 0,
                spdif_d: None,
            };
            set_role(&mut u, *role);
            u.upstream = upstream;
            m.units.push(u);
        }
        E::RemoveElement { id } => remove(m, id)?,
        E::RenameElement { id, name } => {
            text_ok("a name", name)?;
            let slot = m
                .rooms
                .iter_mut()
                .map(|x| (&x.id, &mut x.name))
                .chain(m.zones.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.groups.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.sources.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.senders.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.fan_outs.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.units.iter_mut().map(|x| (&x.id, &mut x.name)))
                .chain(m.scenes.iter_mut().map(|x| (&x.id, &mut x.name)))
                .find(|(i, _)| *i == id);
            match slot {
                Some((_, n)) => n.clone_from(name),
                None => return invalid(format!("no element {id:?}")),
            }
        }
        E::SetUnitRoom { id, room } => {
            if let Some(r) = room {
                need(m, r, Kind::Room, "room")?;
            }
            unit_mut(m, id)?.room.clone_from(room);
        }
        E::SetUnitRole { id, role } => set_role(unit_mut(m, id)?, *role),
        E::SetUnitUpstream {
            id,
            upstream,
            output,
        } => {
            if upstream == id {
                return invalid(format!("unit {id:?} cannot feed itself"));
            }
            if !m.kinds().contains_key(upstream.as_str()) {
                return invalid(format!("no element {upstream:?} to feed it"));
            }
            let u = unit_mut(m, id)?;
            if u.usb_direct() {
                return invalid(format!("unit {id:?} is USB-direct: it has no upstream"));
            }
            u.upstream.clone_from(upstream);
            u.output = *output;
        }
        E::SetUnitUsbMode {
            id,
            mode,
            play_unaligned,
        } => {
            if let Some(md) = mode.as_deref().filter(|md| !USB_MODES.contains(md)) {
                return invalid(format!("USB mode {md:?} (off, rate_lock or phase_lock)"));
            }
            let u = unit_mut(m, id)?;
            u.usb_mode.clone_from(mode);
            u.on_lock_error = if *play_unaligned {
                OnLockError::PlayUnaligned
            } else {
                OnLockError::Mute
            };
        }
        E::SetUnitSpeakers { id, speakers } => {
            let u = unit_mut(m, id)?;
            let old = std::mem::take(&mut u.speakers);
            u.speakers = speakers
                .iter()
                .map(|s| {
                    let kept = old.iter().find(|o| o.output == s.output);
                    Speaker {
                        output: s.output,
                        distance_m: s.distance_m,
                        ..kept.cloned().unwrap_or_default()
                    }
                })
                .collect();
        }
        E::SetSpdifSource { id, tx_mask } => {
            let u = unit_mut(m, id)?;
            u.spdif_source = *tx_mask != 0;
            u.spdif_tx_mask = *tx_mask;
            if *tx_mask == 0 {
                u.spdif_d = None;
            } else if u.spdif() {
                // An S/PDIF source follows no one (one S/PDIF role per unit).
                u.upstream.clear();
            }
        }
        E::AddRoom { id, name } => {
            text_ok("a name", name)?;
            let id = new_id(m, id.as_deref(), name, "room")?;
            m.rooms.push(Room {
                id,
                name: name.clone(),
                lip_sync_us: None,
            });
        }
        E::AddZone {
            id,
            name,
            rooms,
            source,
        } => {
            text_ok("a name", name)?;
            need_all(m, rooms, Kind::Room, "room")?;
            if let Some(s) = source {
                need(m, s, Kind::Source, "source")?;
            }
            let id = new_id(m, id.as_deref(), name, "zone")?;
            m.zones.push(Zone {
                id,
                name: name.clone(),
                rooms: rooms.clone(),
                source: source.clone(),
            });
        }
        E::SetZoneRooms { id, rooms } => {
            need_all(m, rooms, Kind::Room, "room")?;
            match m.zones.iter_mut().find(|z| z.id == *id) {
                Some(z) => z.rooms.clone_from(rooms),
                None => return invalid(format!("no zone {id:?}")),
            }
        }
        E::AddGroup { id, name, units } => {
            text_ok("a name", name)?;
            need_all(m, units, Kind::Unit, "unit")?;
            let id = new_id(m, id.as_deref(), name, "group")?;
            m.groups.push(Group {
                id,
                name: name.clone(),
                units: units.clone(),
                note: String::new(),
            });
        }
        E::SetGroupUnits { id, units } => {
            need_all(m, units, Kind::Unit, "unit")?;
            match m.groups.iter_mut().find(|g| g.id == *id) {
                Some(g) => g.units.clone_from(units),
                None => return invalid(format!("no group {id:?}")),
            }
        }
        E::AddSender { id, name, serial } => {
            text_ok("a name", name)?;
            if let Some(s) = serial {
                serial_ok(m, s)?;
            }
            let from = if name.is_empty() {
                serial.as_deref().unwrap_or("")
            } else {
                name
            };
            let id = new_id(m, id.as_deref(), from, "sender")?;
            m.senders.push(Sender {
                id,
                name: name.clone(),
                serial: serial.clone(),
                sender_id: None,
            });
        }
        E::AddFanOut {
            id,
            name,
            serial,
            upstream,
            output,
        } => {
            text_ok("a name", name)?;
            if let Some(s) = serial {
                serial_ok(m, s)?;
            }
            if !m.kinds().contains_key(upstream.as_str()) {
                return invalid(format!("no element {upstream:?} to feed it"));
            }
            let from = if name.is_empty() {
                serial.as_deref().unwrap_or("")
            } else {
                name
            };
            let id = new_id(m, id.as_deref(), from, "fan-out")?;
            m.fan_outs.push(FanOut {
                id,
                name: name.clone(),
                serial: serial.clone(),
                upstream: upstream.clone(),
                output: *output,
                hop_delay_frames: None,
            });
        }
        E::SetSystem {
            rate_hz,
            set_d,
            d_frames,
            keep_usb_aligned,
        } => {
            if let Some(r) = rate_hz {
                if !matches!(r, 44_100 | 48_000 | 96_000) {
                    return invalid("rateHz: 44100, 48000 or 96000");
                }
                m.rate_hz = *r;
            }
            if *set_d {
                m.d_frames = *d_frames;
            }
            if let Some(k) = keep_usb_aligned {
                m.keep_usb_aligned = *k;
            }
        }
    }
    Ok(())
}

/// Remove element `id` and what names it as a member (see
/// [`InstallationEdit::RemoveElement`]).
fn remove(m: &mut SystemModel, id: &str) -> Result<()> {
    let Some(kind) = m.kinds().get(id).copied() else {
        return invalid(format!("no element {id:?}"));
    };
    let target = match kind {
        Kind::Room => Some(format!("room:{id}")),
        Kind::Zone => Some(format!("zone:{id}")),
        Kind::Group => Some(format!("group:{id}")),
        Kind::Unit => Some(format!("unit:{id}")),
        _ => None,
    };
    match kind {
        Kind::Room => {
            m.rooms.retain(|x| x.id != id);
            for z in &mut m.zones {
                z.rooms.retain(|r| r != id);
            }
            for u in m.units.iter_mut().filter(|u| u.room.as_deref() == Some(id)) {
                u.room = None;
            }
        }
        Kind::Zone => m.zones.retain(|x| x.id != id),
        Kind::Group => m.groups.retain(|x| x.id != id),
        Kind::Source => {
            m.sources.retain(|x| x.id != id);
            for z in m
                .zones
                .iter_mut()
                .filter(|z| z.source.as_deref() == Some(id))
            {
                z.source = None;
            }
        }
        Kind::Sender => {
            m.senders.retain(|x| x.id != id);
            for s in m
                .sources
                .iter_mut()
                .filter(|s| s.sender.as_deref() == Some(id))
            {
                s.sender = None;
            }
        }
        Kind::FanOut => m.fan_outs.retain(|x| x.id != id),
        Kind::Unit => {
            m.units.retain(|x| x.id != id);
            for g in &mut m.groups {
                g.units.retain(|u| u != id);
            }
        }
        Kind::Scene => m.scenes.retain(|x| x.id != id),
    }
    if let Some(t) = target {
        for s in &mut m.scenes {
            s.actions.retain(|a| a.target != t);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------- diff

/// An element's name, else its id; `id` itself when the model lacks it.
fn label(m: &SystemModel, id: &str) -> String {
    let named = m
        .rooms
        .iter()
        .map(|x| (&x.id, &x.name))
        .chain(m.zones.iter().map(|x| (&x.id, &x.name)))
        .chain(m.groups.iter().map(|x| (&x.id, &x.name)))
        .chain(m.sources.iter().map(|x| (&x.id, &x.name)))
        .chain(m.senders.iter().map(|x| (&x.id, &x.name)))
        .chain(m.fan_outs.iter().map(|x| (&x.id, &x.name)))
        .chain(m.units.iter().map(|x| (&x.id, &x.name)))
        .chain(m.scenes.iter().map(|x| (&x.id, &x.name)))
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.as_str());
    match named {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => id.to_owned(),
    }
}

fn opt_label(m: &SystemModel, id: Option<&str>) -> String {
    match id {
        Some(i) if !i.is_empty() => label(m, i),
        _ => "none".into(),
    }
}

fn list_label(m: &SystemModel, ids: &[String]) -> String {
    if ids.is_empty() {
        return "none".into();
    }
    ids.iter()
        .map(|i| label(m, i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A unit's role, in words.
fn role_word(u: &Unit) -> String {
    let base = if u.usb_direct() {
        "USB-direct"
    } else if u.spdif() && u.spdif_source {
        "S/PDIF source"
    } else if u.spdif() {
        "S/PDIF follower"
    } else if u.repeater {
        "repeater"
    } else {
        "receiver"
    };
    if u.spdif_source && !u.spdif() {
        format!("{base} and S/PDIF source")
    } else {
        base.to_owned()
    }
}

fn from_to(out: &mut Vec<String>, what: &str, old: String, new: String) {
    if old != new {
        out.push(format!("{what} {old} → {new}"));
    }
}

/// The changes of one element present in both models, as fragments.
type Changes<'a, T> = &'a dyn Fn(&SystemModel, &T, &SystemModel, &T) -> Vec<String>;

fn list_diff<T: PartialEq>(
    out: &mut Vec<String>,
    kind: &str,
    (bm, base): (&SystemModel, &[T]),
    (dm, draft): (&SystemModel, &[T]),
    id: fn(&T) -> &str,
    changes: Changes<'_, T>,
) {
    let mut word = kind.chars();
    let cap: String = word
        .next()
        .map(|c| c.to_uppercase().chain(word).collect())
        .unwrap_or_default();
    for d in draft {
        match base.iter().find(|b| id(b) == id(d)) {
            None => out.push(format!("Adds {kind} {}", label(dm, id(d)))),
            Some(b) if b != d => {
                let mut parts = changes(bm, b, dm, d);
                if parts.is_empty() {
                    parts.push("settings changed".into());
                }
                out.push(format!("{cap} {}: {}", label(dm, id(d)), parts.join("; ")));
            }
            Some(_) => {}
        }
    }
    for b in base
        .iter()
        .filter(|b| !draft.iter().any(|d| id(d) == id(b)))
    {
        out.push(format!("Removes {kind} {}", label(bm, id(b))));
    }
}

fn name_change(out: &mut Vec<String>, old: &str, new: &str) {
    if old != new {
        out.push(format!("name {old:?} → {new:?}"));
    }
}

/// What saving `draft` over `base` changes, one sentence each: the
/// installation's own settings, then each kind of element (added, changed,
/// removed). Empty when nothing changes.
pub fn installation_changes(base: &SystemModel, draft: &SystemModel) -> Vec<String> {
    let mut out = Vec::new();
    let onoff = |b: bool| if b { "on" } else { "off" }.to_owned();
    let d = |d: Option<u16>| d.map_or_else(|| "automatic".into(), |f| format!("{f} frames"));
    let mut sys = Vec::new();
    from_to(
        &mut sys,
        "sample rate",
        format!("{} Hz", base.rate_hz),
        format!("{} Hz", draft.rate_hz),
    );
    from_to(
        &mut sys,
        "system delay",
        d(base.d_frames),
        d(draft.d_frames),
    );
    from_to(
        &mut sys,
        "lip sync",
        format!("{} µs", base.lip_sync_us),
        format!("{} µs", draft.lip_sync_us),
    );
    from_to(
        &mut sys,
        "apply lead",
        format!("{} frames", base.apply_lead_frames),
        format!("{} frames", draft.apply_lead_frames),
    );
    from_to(
        &mut sys,
        "keep USB-direct units aligned",
        onoff(base.keep_usb_aligned),
        onoff(draft.keep_usb_aligned),
    );
    if !sys.is_empty() {
        out.push(format!("Installation: {}", sys.join("; ")));
    }
    list_diff(
        &mut out,
        "room",
        (base, &base.rooms[..]),
        (draft, &draft.rooms[..]),
        |x: &Room| x.id.as_str(),
        &|_, o, _, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            let ls =
                |v: Option<u32>| v.map_or_else(|| "the system's".into(), |u| format!("{u} µs"));
            from_to(&mut c, "lip sync", ls(o.lip_sync_us), ls(d.lip_sync_us));
            c
        },
    );
    list_diff(
        &mut out,
        "zone",
        (base, &base.zones[..]),
        (draft, &draft.zones[..]),
        |x: &Zone| x.id.as_str(),
        &|bm, o, dm, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            from_to(
                &mut c,
                "rooms",
                list_label(bm, &o.rooms),
                list_label(dm, &d.rooms),
            );
            from_to(
                &mut c,
                "source",
                opt_label(bm, o.source.as_deref()),
                opt_label(dm, d.source.as_deref()),
            );
            c
        },
    );
    list_diff(
        &mut out,
        "group",
        (base, &base.groups[..]),
        (draft, &draft.groups[..]),
        |x: &Group| x.id.as_str(),
        &|bm, o, dm, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            from_to(
                &mut c,
                "units",
                list_label(bm, &o.units),
                list_label(dm, &d.units),
            );
            c
        },
    );
    list_diff(
        &mut out,
        "source",
        (base, &base.sources[..]),
        (draft, &draft.sources[..]),
        |x: &dspi_session::system_model::Source| x.id.as_str(),
        &|_, o, _, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            c
        },
    );
    list_diff(
        &mut out,
        "sender",
        (base, &base.senders[..]),
        (draft, &draft.senders[..]),
        |x: &Sender| x.id.as_str(),
        &|_, o, _, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            c
        },
    );
    list_diff(
        &mut out,
        "fan-out",
        (base, &base.fan_outs[..]),
        (draft, &draft.fan_outs[..]),
        |x: &FanOut| x.id.as_str(),
        &|bm, o, dm, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            from_to(
                &mut c,
                "fed by",
                label(bm, &o.upstream),
                label(dm, &d.upstream),
            );
            c
        },
    );
    list_diff(
        &mut out,
        "unit",
        (base, &base.units[..]),
        (draft, &draft.units[..]),
        |x: &Unit| x.id.as_str(),
        &unit_changes,
    );
    list_diff(
        &mut out,
        "scene",
        (base, &base.scenes[..]),
        (draft, &draft.scenes[..]),
        |x: &dspi_session::system_model::Scene| x.id.as_str(),
        &|_, o, _, d| {
            let mut c = Vec::new();
            name_change(&mut c, &o.name, &d.name);
            from_to(
                &mut c,
                "actions",
                o.actions.len().to_string(),
                d.actions.len().to_string(),
            );
            c
        },
    );
    out
}

fn unit_changes(bm: &SystemModel, o: &Unit, dm: &SystemModel, d: &Unit) -> Vec<String> {
    let mut c = Vec::new();
    name_change(&mut c, &o.name, &d.name);
    from_to(
        &mut c,
        "room",
        opt_label(bm, o.room.as_deref()),
        opt_label(dm, d.room.as_deref()),
    );
    from_to(&mut c, "role", role_word(o), role_word(d));
    from_to(
        &mut c,
        "fed by",
        opt_label(bm, Some(&o.upstream)),
        opt_label(dm, Some(&d.upstream)),
    );
    let mode = |u: &Unit| u.usb_mode.clone().unwrap_or_else(|| "its own".into());
    from_to(&mut c, "USB mode", mode(o), mode(d));
    from_to(
        &mut c,
        "in lock error",
        o.on_lock_error.as_str().replace('_', " "),
        d.on_lock_error.as_str().replace('_', " "),
    );
    from_to(
        &mut c,
        "latency",
        format!("{} µs", o.latency()),
        format!("{} µs", d.latency()),
    );
    if o.speakers.len() != d.speakers.len() {
        from_to(
            &mut c,
            "speakers",
            o.speakers.len().to_string(),
            d.speakers.len().to_string(),
        );
    } else if o.speakers != d.speakers {
        c.push("speakers changed".into());
    }
    if o.slots != d.slots {
        c.push("slots changed".into());
    }
    c
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
#[path = "tests_wp_inst0.rs"]
mod tests_wp_inst0;
