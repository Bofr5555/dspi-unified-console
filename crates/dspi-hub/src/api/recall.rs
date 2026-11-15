//! `snapshot.recall` on the hub (WP P2PA; `hub-api.md` section 11.2): a snapshot from the
//! hub's own store (P2SS), written through the executor's core
//! (`DspiCore::snapshot_recall_document`: the image and the settings outside it, in a
//! recall's order) as the caller's child session.
//!
//! - **P1HL's rule holds** because the hub judges the child as any client: on a device
//!   with a limits profile or a lock a Control caller's image is `image_refused` (nothing
//!   is written); an Admin's is written, then held to the profile after the fact.
//! - **`guard: true`** begins a guarded apply (P1HA) first, as the caller, so the caller
//!   confirms it with `apply.confirm`, and no answer in time, a cancel or the caller's
//!   session ending writes the captured state back. A recall that fails after its guard
//!   began is reverted at once.
//! - **A device that is not attached** gets nothing: `offline`, before anything is opened
//!   or sent.
//! - **History:** the recall is put on the bus as an unstored `Event` record (`id` 0,
//!   kind `recall`, its `lines`, the child session and the caller's name in `by`), which
//!   the hub's history (P2EV) files.

use std::sync::Arc;
use std::time::Duration;

use dspi_session::events::{EventKind, EventRecord};
use serde_json::{Value, json};

use super::exec::Executor;
use super::{ApiError, no_foreign_guard, require, target};
use crate::apply_guard::DEFAULT_TIMEOUT;
use crate::bus::BusMessage;
use crate::hub::HubHandle;
use crate::session::{Role, Session};
use crate::snapshots::HubSnapshots;

pub(super) fn handle(
    exec: &Executor,
    store: Option<&HubSnapshots>,
    hub: &HubHandle,
    session: &Arc<Session>,
    msg: &Value,
) -> Result<Value, ApiError> {
    require(session, Role::Control)?;
    let store = store.ok_or_else(|| {
        ApiError::new(
            "unsupported",
            "this hub keeps no snapshots (it has no data folder)",
        )
    })?;
    let (serial, handle) = target(hub, msg)?;
    let (id, text) = pick(store, &serial, msg)?;
    let guard = match msg.get("guard") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::bad("guard is true or false")),
    };
    let timeout = match msg.get("timeout_ms") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(v) => Duration::from_millis(
            v.as_u64()
                .ok_or_else(|| ApiError::bad("timeout_ms is a whole number of milliseconds"))?,
        ),
    };
    no_foreign_guard(hub, handle, session.id())?;
    let begun = if guard {
        let b = hub
            .apply_begin(session, handle, timeout)
            .map_err(|e| ApiError::new(e.code(), format!("the guard could not begin: {e:?}")))?;
        Some(b.guard)
    } else {
        None
    };
    let done = exec.run(hub, session, &serial, |cx| {
        let core = cx.core()?;
        let r = core
            .snapshot_recall_document(text)
            .map_err(ApiError::from)?;
        let mut record = EventRecord::new(
            EventKind::Recall,
            hub.clock().unix_ms(),
            format!("Recalled snapshot {} on the hub", r.name),
        );
        record.lines = r.changes.clone();
        record.session = Some(u32::from(cx.child_id()));
        record.by = cx.by();
        record.source = Some("hub".into());
        hub.bus().publish(BusMessage::Event {
            serial: cx.serial().to_owned(),
            record,
        });
        Ok(json!({
            "serial": cx.serial(),
            "snapshot": id,
            "name": r.name,
            "image": r.image,
            "lines": r.changes,
            "notes": r.notes,
        }))
    });
    match (done, begun) {
        (Ok(mut body), Some(g)) => {
            if let Some(m) = body.as_object_mut() {
                m.insert("guard".into(), json!(g));
            }
            Ok(body)
        }
        (Ok(body), None) => Ok(body),
        (Err(e), Some(_)) => {
            // Back to what the guard captured, now, rather than at its deadline.
            if let Err(c) = hub.apply_cancel(session, handle) {
                log::warn!("hub api: a failed recall's guard did not revert: {c:?}");
            }
            Err(e)
        }
        (Err(e), None) => Err(e),
    }
}

/// The snapshot a request names, `snapshot` (its id) or `name` (exact; two of `serial`'s
/// snapshots with it are `conflict`), and its document's text. A snapshot of another
/// device is `not_found`.
fn pick(store: &HubSnapshots, serial: &str, msg: &Value) -> Result<(String, String), ApiError> {
    let from_store = |e: crate::snapshots::SnapshotError| ApiError::new(e.code(), e.to_string());
    let id = match (
        msg.get("snapshot").and_then(Value::as_str),
        msg.get("name").and_then(Value::as_str),
    ) {
        (Some(id), _) => id.to_owned(),
        (None, Some(name)) => {
            let found: Vec<String> = store
                .list(serial)
                .map_err(from_store)?
                .into_iter()
                .filter(|e| e.name == name)
                .map(|e| e.id)
                .collect();
            match found.as_slice() {
                [id] => id.clone(),
                [] => {
                    return Err(ApiError::new(
                        "not_found",
                        format!("no snapshot of {serial} is named {name}"),
                    ));
                }
                _ => {
                    return Err(ApiError::new(
                        "conflict",
                        format!(
                            "{} snapshots are named {name}; recall one by id",
                            found.len()
                        ),
                    ));
                }
            }
        }
        (None, None) => return Err(ApiError::bad("snapshot or name is required")),
    };
    let (entry, text) = store.get(&id).map_err(from_store)?;
    if entry.serial != serial {
        return Err(ApiError::new(
            "not_found",
            format!("snapshot {id} is not one of {serial}'s"),
        ));
    }
    Ok((id, text))
}
