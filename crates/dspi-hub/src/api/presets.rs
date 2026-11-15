//! `preset.list`, `preset.load`, `preset.save` (WP P2PA; `hub-api.md` section 10), on
//! the executor's core: `get_preset_directory`, `load_preset` (held after the fact on a
//! device with a limits profile, as P1HL does), `save_preset_confirmed` with a token the
//! hub's core mints for an Admin's `"confirm": true`.

use std::sync::Arc;

use dspi_core::ConfirmOp;
use serde_json::{Value, json};

use super::exec::{Cx, Executor};
use super::{ApiError, no_foreign_guard, require, target};
use crate::hub::HubHandle;
use crate::session::{Role, Session};

/// Preset slots on every DSPi (`MAX_PRESETS`).
pub const SLOTS: u8 = 10;
/// A preset name's most, in bytes (the firmware's 32 with its NUL).
const NAME_BYTES: usize = 31;

pub(super) fn handle(
    exec: &Executor,
    hub: &HubHandle,
    session: &Arc<Session>,
    t: &str,
    msg: &Value,
) -> Result<Value, ApiError> {
    let (serial, handle) = target(hub, msg)?;
    match t {
        "preset.list" => exec.run(hub, session, &serial, list),
        "preset.load" => {
            require(session, Role::Control)?;
            no_foreign_guard(hub, handle, session.id())?;
            exec.run(hub, session, &serial, |cx| {
                let slot = slot_of(cx, msg)?;
                let core = cx.core()?;
                core.load_preset(slot).map_err(ApiError::from)?;
                let name = names(cx)?
                    .get(usize::from(slot))
                    .cloned()
                    .unwrap_or_default();
                Ok(json!({"serial": cx.serial(), "slot": slot, "name": name}))
            })
        }
        // "preset.save"
        _ => {
            require(session, Role::Admin)?;
            if msg.get("confirm").and_then(Value::as_bool) != Some(true) {
                return Err(ApiError::new(
                    "confirmation_required",
                    "preset.save writes flash: it is done only with \"confirm\": true",
                ));
            }
            let slot = slot_number(msg.get("slot"))?;
            let name = match msg.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(n)) if n.len() <= NAME_BYTES => Some(n.clone()),
                Some(_) => {
                    return Err(ApiError::bad(format!(
                        "name is text of at most {NAME_BYTES} bytes"
                    )));
                }
            };
            no_foreign_guard(hub, handle, session.id())?;
            exec.run(hub, session, &serial, |cx| {
                let core = cx.core()?;
                let token = core.confirm_token(ConfirmOp::SavePreset);
                core.save_preset_confirmed(slot, Some(token))
                    .map_err(ApiError::from)?;
                if let Some(name) = &name {
                    core.set_preset_name(slot, name.clone())
                        .map_err(ApiError::from)?;
                }
                let name = names(cx)?
                    .get(usize::from(slot))
                    .cloned()
                    .unwrap_or_default();
                Ok(json!({"serial": cx.serial(), "slot": slot, "name": name}))
            })
        }
    }
}

fn bit(slot: u8) -> u16 {
    1u16.checked_shl(u32::from(slot)).unwrap_or(0)
}

fn slot_number(v: Option<&Value>) -> Result<u8, ApiError> {
    v.and_then(Value::as_u64)
        .and_then(|s| u8::try_from(s).ok())
        .filter(|s| *s < SLOTS)
        .ok_or_else(|| {
            ApiError::bad(format!(
                "slot is a whole number 0 to {}",
                SLOTS.saturating_sub(1)
            ))
        })
}

/// The slot names, one per slot: each used slot's read from the device (another
/// session may have named it since this core attached), the mirror's where a read
/// fails, "" for an empty slot.
fn names(cx: &mut Cx<'_>) -> Result<Vec<String>, ApiError> {
    let core = cx.core()?;
    let used = core
        .get_preset_directory()
        .map_err(ApiError::from)?
        .occupied_mask;
    let mut names = core.get_state().preset_names;
    names.resize(usize::from(SLOTS), String::new());
    for (slot, name) in (0..SLOTS).zip(names.iter_mut()) {
        if used & bit(slot) == 0 {
            name.clear();
        } else if let Ok(read) = core.get_preset_name(slot) {
            *name = read;
        }
    }
    Ok(names)
}

/// The slot a `preset.load` names: `slot`, or `name` (exact, case kept; two slots with
/// it are `conflict`, none `not_found`).
fn slot_of(cx: &mut Cx<'_>, msg: &Value) -> Result<u8, ApiError> {
    match (msg.get("slot"), msg.get("name").and_then(Value::as_str)) {
        (Some(s), _) if !s.is_null() => slot_number(Some(s)),
        (_, Some(name)) => {
            let used = cx
                .core()?
                .get_preset_directory()
                .map_err(ApiError::from)?
                .occupied_mask;
            let found: Vec<u8> = names(cx)?
                .iter()
                .zip(0u8..)
                .filter(|(n, s)| n.as_str() == name && used & bit(*s) != 0)
                .map(|(_, s)| s)
                .collect();
            match found.as_slice() {
                [slot] => Ok(*slot),
                [] => Err(ApiError::new(
                    "not_found",
                    format!("no preset is named {name}"),
                )),
                _ => Err(ApiError::new(
                    "conflict",
                    format!("{} presets are named {name}; load one by slot", found.len()),
                )),
            }
        }
        _ => Err(ApiError::bad("slot or name is required")),
    }
}

fn list(cx: &mut Cx<'_>) -> Result<Value, ApiError> {
    let core = cx.core()?;
    let dir = core.get_preset_directory().map_err(ApiError::from)?;
    let names = names(cx)?;
    let used = |s: u8| dir.occupied_mask & bit(s) != 0;
    let slots: Vec<Value> = (0..SLOTS)
        .zip(names.iter())
        .map(|(s, n)| json!({"slot": s, "name": n, "used": used(s)}))
        .collect();
    let active = (dir.last_active < SLOTS && used(dir.last_active)).then_some(dir.last_active);
    let startup = match dir.startup_mode {
        0 => json!({"mode": "slot", "slot": dir.default_slot}),
        _ => json!({"mode": "last_active"}),
    };
    Ok(json!({"serial": cx.serial(), "slots": slots, "active": active, "startup": startup}))
}
