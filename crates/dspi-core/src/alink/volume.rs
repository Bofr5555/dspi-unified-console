//! WP FEAT2: the volume of a room or a zone of the installation, as one live
//! control. The hub keeps the site (`system.get`) and has one fan-out for a
//! parameter (`group.param.set`, hub-api.md section 20.2); this is the two
//! together, for a System screen, the Terminal and the AI agents' tool.
//!
//! - [`DspiCore::system_volume_plan`] reads: the units of the room or zone,
//!   each unit's volume now and what it would become, what stands in the way
//!   of a unit, and whether any unit gets louder. Nothing is written.
//! - [`DspiCore::system_volume_set`] writes a plan: `group.param.set` on the
//!   units that may be written, one after another, each judged on its own by
//!   the hub (the role, that unit's limits profile, its lock). One unit's
//!   refusal stops no other; every unit has its row.
//!
//! What is written is the plan's absolute value for each unit, so a plan the
//! user agreed to never writes another level than the one it showed. A unit
//! whose volume could not be read is not written: whether it gets louder is
//! not known. This console's limits profile for a unit (a master volume
//! ceiling) refuses that unit too, as it refuses a direct write.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use dspi_session::system_model::SystemModel;

use crate::hub_server::{HubFleetTarget, HubGroupRow, HubOffset};
use crate::{DspiCore, DspiError, ParamValue, Result};

/// The hub's most serials in one fan-out.
const FAN_OUT: usize = 16;
/// The user volume's range, dB (the registry's).
pub const SYSTEM_VOLUME_USER_MIN_DB: f32 = -60.0;
/// The master volume's lowest level, dB (-128 is its mute, not a level).
pub const SYSTEM_VOLUME_MASTER_MIN_DB: f32 = -127.0;
/// The largest step either way, dB (the user volume's whole range).
pub const SYSTEM_VOLUME_STEP_MAX_DB: f32 = 60.0;
/// A volume change smaller than this is no change, dB.
const SAME_DB: f32 = 0.05;

/// Which volume of the units.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemVolumeKind {
    /// The listening volume (`vol.user`, -60 to 0 dB).
    #[default]
    User,
    /// The master volume (`vol.master`, -127 to 0 dB in 0.5 dB steps).
    Master,
}

impl SystemVolumeKind {
    fn path(self) -> &'static str {
        match self {
            Self::User => "vol.user",
            Self::Master => "vol.master",
        }
    }

    fn min_db(self) -> f32 {
        match self {
            Self::User => SYSTEM_VOLUME_USER_MIN_DB,
            Self::Master => SYSTEM_VOLUME_MASTER_MIN_DB,
        }
    }

    /// `db` in the volume's range, on its grid.
    fn fit(self, db: f32) -> f32 {
        let db = db.clamp(self.min_db(), 0.0);
        match self {
            Self::User => db,
            Self::Master => (db * 2.0).round() / 2.0,
        }
    }
}

/// What to set: exactly one of `room` and `zone`, one of `level_db` and
/// `step_db`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemVolumeAsk {
    /// A room's id in the installation; "" for a zone.
    pub room: String,
    /// A zone's id (every unit of its rooms); "" for a room.
    pub zone: String,
    /// Which volume.
    pub volume: SystemVolumeKind,
    /// The same level on every unit, dB.
    pub level_db: Option<f32>,
    /// Each unit's own volume moved by this, dB (negative is quieter).
    pub step_db: Option<f32>,
}

/// Where one unit stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemVolumeState {
    /// It would be written.
    Ready,
    /// It is at the level asked for: nothing to write.
    Unchanged,
    /// It may not be, or could not be, written (`code` and `message` say why).
    #[default]
    Refused,
    /// Written; `after_db` is what it has now.
    Done,
}

/// One unit of the room or zone.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemVolumeUnit {
    /// The installation's id of the unit.
    pub unit_id: String,
    /// Its name, or the id when it has none.
    pub name: String,
    /// Its serial.
    pub serial: String,
    /// Its volume now, dB, when it could be read.
    pub before_db: Option<f32>,
    /// What it would be set to, dB.
    pub target_db: Option<f32>,
    /// What it was set to, dB, once written.
    pub after_db: Option<f32>,
    /// It gets louder.
    pub louder: bool,
    /// Where it stands.
    pub state: SystemVolumeState,
    /// Why it was refused: the hub's code (`offline`, `limit_exceeded`,
    /// `denied`, `locked`, ...), "" when not.
    pub code: String,
    /// The same in words.
    pub message: String,
}

impl SystemVolumeUnit {
    /// Refuse the unit with `code` and `message`.
    pub fn refuse(&mut self, code: &str, message: &str) {
        self.state = SystemVolumeState::Refused;
        self.code = code.to_owned();
        self.message = message.to_owned();
    }
}

/// A room's or zone's volume, planned ([`DspiCore::system_volume_plan`]) or
/// set ([`DspiCore::system_volume_set`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemVolumePlan {
    /// What was asked.
    pub ask: SystemVolumeAsk,
    /// The room's or zone's name, or its id when it has none.
    pub name: String,
    /// Its place in the installation's list, from 1.
    pub number: u32,
    /// How many rooms (or zones) the installation has.
    pub of: u32,
    /// Its units, in the installation's order.
    pub units: Vec<SystemVolumeUnit>,
}

impl SystemVolumePlan {
    /// The units that would be written.
    pub fn ready(&self) -> impl Iterator<Item = &SystemVolumeUnit> {
        self.units
            .iter()
            .filter(|u| u.state == SystemVolumeState::Ready)
    }

    /// Whether any unit that would be written gets louder.
    pub fn louder(&self) -> bool {
        self.ready().any(|u| u.louder)
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// The ask's target (`room:<id>`, `zone:<id>`), checked.
fn target(ask: &SystemVolumeAsk) -> Result<(&'static str, &str)> {
    match (ask.room.is_empty(), ask.zone.is_empty()) {
        (false, true) => Ok(("room", &ask.room)),
        (true, false) => Ok(("zone", &ask.zone)),
        _ => Err(invalid("name one of a room or a zone")),
    }
}

/// The ask's numbers, checked.
fn check_amount(ask: &SystemVolumeAsk) -> Result<()> {
    let min = ask.volume.min_db();
    match (ask.level_db, ask.step_db) {
        (Some(l), None) if l.is_finite() && (min..=0.0).contains(&l) => Ok(()),
        (Some(l), None) => Err(invalid(format!("a level of {l} dB ({min} to 0 dB)"))),
        (None, Some(s)) if s.is_finite() && s.abs() <= SYSTEM_VOLUME_STEP_MAX_DB => Ok(()),
        (None, Some(s)) => Err(invalid(format!(
            "a step of {s} dB (at most {SYSTEM_VOLUME_STEP_MAX_DB} dB either way)"
        ))),
        _ => Err(invalid("give one of a level or a step")),
    }
}

/// What [`SystemVolumeAsk`] makes of a unit at `before`.
fn target_db(ask: &SystemVolumeAsk, before: f32) -> f32 {
    ask.volume.fit(match ask.step_db {
        Some(s) => before + s,
        None => ask.level_db.unwrap_or(before),
    })
}

/// The units of `ask` in `model`, each unit once, with the target's name and
/// place.
pub(crate) fn units_of(model: &SystemModel, ask: &SystemVolumeAsk) -> Result<SystemVolumePlan> {
    let (kind, id) = target(ask)?;
    let names: Vec<(&str, &str)> = match kind {
        "room" => model.rooms.iter().map(|r| (&*r.id, &*r.name)).collect(),
        _ => model.zones.iter().map(|z| (&*z.id, &*z.name)).collect(),
    };
    let at = names.iter().position(|(i, _)| *i == id);
    let found = at.and_then(|a| names.get(a)).copied();
    let (Some(at), Some((_, name))) = (at, found) else {
        return Err(invalid(format!("the installation has no such {kind}")));
    };
    let mut seen = BTreeSet::new();
    let units: Vec<SystemVolumeUnit> = model
        .resolve_target(&format!("{kind}:{id}"))
        .unwrap_or_default()
        .into_iter()
        .filter(|u| !u.serial.is_empty() && seen.insert(u.serial.clone()))
        .map(|u| SystemVolumeUnit {
            unit_id: u.id.clone(),
            name: or_id(&u.name, &u.id),
            serial: u.serial.clone(),
            ..SystemVolumeUnit::default()
        })
        .collect();
    if units.is_empty() {
        return Err(invalid(format!("the {kind} has no units")));
    }
    Ok(SystemVolumePlan {
        ask: ask.clone(),
        name: or_id(name, id),
        number: u32::try_from(at.saturating_add(1)).unwrap_or(u32::MAX),
        of: u32::try_from(names.len()).unwrap_or(u32::MAX),
        units,
    })
}

fn or_id(name: &str, id: &str) -> String {
    if name.is_empty() { id } else { name }.to_owned()
}

/// `row`'s number.
fn number(row: &HubGroupRow) -> Option<f32> {
    match row.value {
        Some(ParamValue::Float(f)) if f.is_finite() => Some(f),
        #[allow(clippy::cast_precision_loss)] // a level in dB
        Some(ParamValue::Int(i)) => Some(i as f32),
        _ => None,
    }
}

/// Judge one unit read at `before`.
pub(crate) fn weigh(ask: &SystemVolumeAsk, unit: &mut SystemVolumeUnit, before: f32) {
    unit.before_db = Some(before);
    // The master volume's own mute is no level: a step from it would be one.
    if ask.volume == SystemVolumeKind::Master
        && before < SYSTEM_VOLUME_MASTER_MIN_DB
        && ask.step_db.is_some()
    {
        unit.refuse(
            "muted",
            "its master volume is muted (-128): give a level instead of a step",
        );
        return;
    }
    let to = target_db(ask, before);
    unit.target_db = Some(to);
    unit.louder = to > before + SAME_DB;
    unit.state = if (to - before).abs() < SAME_DB {
        SystemVolumeState::Unchanged
    } else {
        SystemVolumeState::Ready
    };
}

/// The rows of `results` by serial.
fn by_serial(results: Vec<HubGroupRow>) -> BTreeMap<String, HubGroupRow> {
    results.into_iter().map(|r| (r.serial.clone(), r)).collect()
}

/// A hub row's refusal on `unit`.
fn refused_row(unit: &mut SystemVolumeUnit, row: Option<&HubGroupRow>) {
    match row {
        Some(r) if !r.code.is_empty() => unit.refuse(&r.code, &r.msg),
        _ => unit.refuse("no_answer", "the hub gave no answer for it"),
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// What setting a room's or zone's volume would do (`system.get`, then
    /// `group.param.get` on its units): each unit's volume now, its new one,
    /// whether it gets louder, or why it is left alone. Writes nothing. Every
    /// role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an ask that is not one, a
    /// room or zone the installation does not hold or one with no units
    /// (nothing is read from the units); [`DspiError::Unsupported`] without
    /// the hub's `group_params` cap; [`DspiError::Disconnected`]; the hub's
    /// refusal.
    pub fn system_volume_plan(&self, ask: SystemVolumeAsk) -> Result<SystemVolumePlan> {
        target(&ask)?;
        check_amount(&ask)?;
        let got = self.link_hub_request("system.get", Map::new())?;
        let model = match got.get("model") {
            None | Some(Value::Null) => SystemModel::default(),
            Some(m) => serde_json::from_value(m.clone()).map_err(|e| DspiError::Link {
                code: "protocol".into(),
                msg: format!("the hub's installation could not be read: {e}"),
            })?,
        };
        let mut plan = units_of(&model, &ask)?;
        let path = ask.volume.path();
        for chunk in plan.units.chunks_mut(FAN_OUT) {
            let rows = by_serial(
                self.hub_group_param_get(
                    HubFleetTarget {
                        serials: chunk.iter().map(|u| u.serial.clone()).collect(),
                        ..HubFleetTarget::default()
                    },
                    path.to_owned(),
                    Vec::new(),
                )?
                .results,
            );
            for unit in chunk {
                let row = rows.get(&unit.serial);
                match row.filter(|r| r.ok).and_then(number) {
                    Some(before) => weigh(&ask, unit, before),
                    None => refused_row(unit, row),
                }
            }
        }
        if ask.volume == SystemVolumeKind::Master {
            for unit in &mut plan.units {
                let Some(to) = unit
                    .target_db
                    .filter(|_| unit.state == SystemVolumeState::Ready)
                else {
                    continue;
                };
                let held = crate::limits::profile_of(&mut self.lock(), &unit.serial)
                    .and_then(|p| p.check_master_volume(to).err());
                if let Some(v) = held {
                    unit.refuse(v.code, &v.msg);
                }
            }
        }
        Ok(plan)
    }

    /// Write `plan` (from [`DspiCore::system_volume_plan`]): its units that
    /// are ready, through the hub's fan-out (`group.param.set`), each to its
    /// `target_db`. The answer is the plan with each unit's row settled: done
    /// with the level it has, or refused by the hub for that unit (its limits
    /// profile, its lock, offline, the role). Control or Admin on the hub.
    /// Not in the undo history: it goes to the hub, not to this device. Slow:
    /// off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a plan with a level out of
    /// range or a unit twice (nothing sent); [`DspiError::ReadOnly`] (nothing
    /// sent); [`DspiError::Denied`] for a Viewer; as
    /// [`DspiCore::system_volume_plan`]. When the session goes between two
    /// fan-outs, the units already written are done and the rest refused.
    pub fn system_volume_set(&self, plan: SystemVolumePlan) -> Result<SystemVolumePlan> {
        let mut plan = plan;
        let volume = plan.ask.volume;
        let mut ready: Vec<(usize, f32)> = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, u) in plan.units.iter().enumerate() {
            if u.state != SystemVolumeState::Ready {
                continue;
            }
            let to = u
                .target_db
                .filter(|t| t.is_finite() && (volume.min_db()..=0.0).contains(t))
                .ok_or_else(|| invalid("a unit's level is out of range"))?;
            if !seen.insert(u.serial.clone()) {
                return Err(invalid("a unit twice"));
            }
            ready.push((i, to));
        }
        let mut written = false;
        for chunk in ready.chunks(FAN_OUT) {
            let wanted: Vec<(String, f32)> = chunk
                .iter()
                .filter_map(|(i, to)| plan.units.get(*i).map(|u| (u.serial.clone(), *to)))
                .collect();
            // One value for the fan-out, and each unit's own as its offset.
            let base = wanted.iter().map(|(_, t)| *t).fold(0.0, f32::min);
            let answer = self.hub_group_param_set(
                HubFleetTarget {
                    serials: wanted.iter().map(|(s, _)| s.clone()).collect(),
                    ..HubFleetTarget::default()
                },
                volume.path().to_owned(),
                Vec::new(),
                ParamValue::Float(base),
                wanted
                    .iter()
                    .filter(|(_, t)| (t - base).abs() > f32::EPSILON)
                    .map(|(serial, t)| HubOffset {
                        serial: serial.clone(),
                        db: f64::from(*t) - f64::from(base),
                    })
                    .collect(),
            );
            let rows = match answer {
                Ok(a) => by_serial(a.results),
                // Some units may have it already: say so for the rest.
                Err(e) if written => {
                    for (i, _) in chunk {
                        if let Some(u) = plan.units.get_mut(*i) {
                            u.refuse(&e.notice_key(), &e.to_string());
                        }
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };
            written = true;
            for (i, to) in chunk {
                let Some(u) = plan.units.get_mut(*i) else {
                    continue;
                };
                let row = rows.get(&u.serial);
                match row.filter(|r| r.ok) {
                    Some(r) => {
                        u.state = SystemVolumeState::Done;
                        u.after_db = Some(number(r).unwrap_or(*to));
                    }
                    None => refused_row(u, row),
                }
            }
        }
        Ok(plan)
    }
}
