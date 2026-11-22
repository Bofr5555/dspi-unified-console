//! C functions: setting up the installation the hub keeps (WP INST0, MPLAN
//! 1.4), over `dspi_core::link::alink_installation` (feature `link`).
//!
//! A draft is JSON `{revision, base, model}`: the hub's model as read at
//! `revision` (`base`) and with the edits (`model`), each the hub's own JSON
//! (`docs/integrator/audio-link.md`). The client keeps it between calls;
//! nothing but `dspi_installation_view` and `dspi_installation_save` sends
//! anything. Edits are a JSON list of `{op, ...}` (`dspi_core::link::
//! InstallationEdit`, camelCase): `addUnit {serial, id, name, room, role
//! (receiver, repeater, spdif_follower, usb_direct), upstream}`,
//! `removeElement {id}`, `renameElement {id, name}`, `setUnitRoom {id,
//! room}`, `setUnitRole {id, role}`, `setUnitUpstream {id, upstream,
//! output}`, `setUnitUsbMode {id, mode, playUnaligned}`, `setUnitSpeakers
//! {id, speakers: [{output, distanceM}]}`, `setSpdifSource {id, txMask}`,
//! `addRoom {id, name}`, `addZone {id, name, rooms, source}`, `setZoneRooms
//! {id, rooms}`, `addGroup {id, name, units}`, `setGroupUnits {id, units}`,
//! `addSender {id, name, serial}`, `addFanOut {id, name, serial, upstream,
//! output}`, `setSystem {rateHz, setD, dFrames, keepUsbAligned}`.
//!
//! Replies as in the parent module's docs. The view and the drafts carry two
//! whole models: give a buffer of 64 KiB, and ask again with the length
//! returned when it was short. A save whose reply did not fit **has
//! happened**: read the view again rather than saving again (a second save of
//! the same draft is refused with `conflict` anyway).

use std::ffi::c_char;

use dspi_core::link::{InstallationDraft, InstallationEdit};
use dspi_core::{DspiError, Result};
use serde_json::Value;

use super::{arg, json_reply, text};

/// `draft_json` as a draft.
fn draft_arg(draft_json: *const c_char) -> Result<InstallationDraft> {
    let v: Value = serde_json::from_str(arg(draft_json, "draft_json")?).map_err(|e| {
        DspiError::InvalidArgument {
            msg: format!("draft_json: {e}"),
        }
    })?;
    InstallationDraft::from_json(&v)
}

/// The installation on the hub this session is on: JSON {ok, revision, role,
/// mayEdit, draft: {revision, base, model}, unseen: [{serial, name, online,
/// fw, outputs}], inventory} (`inventory` false: the hub keeps no inventory,
/// so `unseen` is empty without meaning "none"). Every role. Blocks on the
/// network: off the UI thread. Failures: `disconnected`, `link` with the
/// hub's code.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installation_view(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(c.installation_view().and_then(|v| {
            let mut out = serde_json::to_value(&v).map_err(|e| DspiError::Io {
                msg: format!("the reply could not be encoded: {e}"),
            })?;
            if let Some(m) = out.as_object_mut() {
                m.insert("draft".into(), v.draft.to_json());
            }
            Ok(out)
        }))
    })
}

/// `edits_json` (a JSON list of edits, module docs) made on `draft_json`,
/// all or none: the new draft, JSON {ok, revision, base, model}, or a
/// failure `invalid` naming the edit (`edit 2: no room "x"`). Sends nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installation_draft_edit(
    core: *mut crate::FfiCore,
    draft_json: *const c_char,
    edits_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let edits = || -> Result<Vec<InstallationEdit>> {
            serde_json::from_str(arg(edits_json, "edits_json")?).map_err(|e| {
                DspiError::InvalidArgument {
                    msg: format!("edits_json: {e}"),
                }
            })
        };
        json_reply(
            draft_arg(draft_json)
                .and_then(|d| c.installation_draft_edit(d, edits()?))
                .map(|d| d.to_json()),
        )
    })
}

/// The hub's own validator on `draft_json`: JSON {ok, value: [{subject,
/// message}]}, empty when the hub would take it (`subject`: the element's
/// id, "" for the installation as a whole). Sends nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installation_validate(
    core: *mut crate::FfiCore,
    draft_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(draft_arg(draft_json).and_then(|d| c.installation_validate(d)))
    })
}

/// What saving `draft_json` would change: JSON {ok, value: [sentence, ...]},
/// empty for nothing. Sends nothing.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installation_diff(
    core: *mut crate::FfiCore,
    draft_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(draft_arg(draft_json).and_then(|d| c.installation_diff(d)))
    })
}

/// Save `draft_json` as the installation (Admin): one `system.set` at the
/// draft's revision. JSON {ok, revision, warnings} (the hub's warnings about
/// the model it kept), or a failure: `invalid` for a draft the hub would
/// refuse (nothing sent), `read_only` (nothing sent), `link` with noticeKey
/// `dlink.conflict` when the installation changed on the hub meanwhile (read
/// the view again), or `dlink` with the hub's words for another role.
/// Blocks: off the UI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_installation_save(
    core: *mut crate::FfiCore,
    draft_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(draft_arg(draft_json).and_then(|d| c.installation_save(d)))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[path = "tests_wp_inst0.rs"]
mod tests_wp_inst0;
