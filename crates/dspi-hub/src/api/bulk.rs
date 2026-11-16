//! Bulk apply (WP P3BA, PLAN-P3 F5, OD-P3d; `hub-api.md` section 20.3): one snapshot or
//! document applied to every member of a group (or a `targets` list), each with its own
//! way back.
//!
//! - **The source.** `snapshot` (an id of the hub's store, of any device), `name` (exactly
//!   one snapshot of the whole store; two are `conflict`) or `document` (its text or the
//!   JSON object, one frame at most). The same text goes to every member, applied as the
//!   core applies a document from another device: its wire format and output count decide
//!   each row (`unsupported` when that device's core refuses it).
//! - **Each member** is `snapshot.recall`'s path (section 11.2, `recall.rs`): the executor's
//!   core over the caller's **child session** (`DspiCore::snapshot_recall_document`), so
//!   P1HL's rule holds per device (`image_refused` for Control on a limited device, held
//!   after the fact for Admin), and a `recall` record in that device's history names the
//!   caller in `by`. A device with any pending guarded apply, the caller's own included,
//!   is that row's `conflict` and gets nothing.
//! - **`guard: true`**: one P1HA guard per device, begun as the caller immediately before
//!   that device's write and reverting to that device's own captured state. All of them
//!   belong to the caller's session, so its closing reverts what is pending.
//!   `group.apply.confirm {run}` keeps every guard of the run still pending,
//!   `group.apply.cancel {run}` reverts them now; a settled guard answers its row again.
//! - **Sequential** (OD-P3d): the answer comes after the last member. A row whose guard
//!   ran out before the run ended carries `reverted: true` and the answer's `msg` says to
//!   raise `timeout_ms`.
//! - **Bounded:** 16 `targets`, a group's 64 members, 256 rows (a request past it is
//!   `bad_request`, nothing sent), [`MAX_RUNS`] runs kept, one group request per session
//!   at a time (`busy`). Never retried. No image bytes in a log line.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use dspi_session::events::{EventKind, EventRecord};
use serde_json::{Map, Value, json};

use super::exec::Executor;
use super::{ApiError, reply, require};
use crate::apply_guard::{ApplyRefused, EndResult};
use crate::bus::BusMessage;
use crate::groups::{GroupResolver, MAX_RESULT_ROWS};
use crate::hub::HubHandle;
use crate::registry::DeviceState;
use crate::service::{HubService, ServiceSetup};
use crate::session::{Role, Session, SessionId};
use crate::snapshots::{HubSnapshots, MAX_WIRE_DOC_BYTES};

/// The `hello` cap of bulk apply (section 20.3).
pub const CAP: &str = "group_apply";
/// The messages of this family.
pub const MESSAGES: [&str; 3] = ["group.apply", "group.apply.confirm", "group.apply.cancel"];
/// The most serials in `targets`.
pub const MAX_TARGETS: usize = 16;
/// The runs kept for `confirm` and `cancel`; past it the oldest settled one goes.
pub const MAX_RUNS: usize = 64;
/// The shortest `timeout_ms`.
pub const MIN_TIMEOUT_MS: u64 = 1_000;
/// The longest `timeout_ms` (the guard's own cap).
pub const MAX_TIMEOUT_MS: u64 = 600_000;
/// `timeout_ms` when the request gives none.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The service, on a hub with groups and a snapshot store (section 19.1's table).
pub(crate) fn service(
    setup: &ServiceSetup<'_>,
    store: Option<Arc<HubSnapshots>>,
) -> Option<Arc<dyn HubService>> {
    let groups = setup.group_resolver()?;
    let store = store?;
    Some(Arc::new(BulkService {
        exec: Executor::new(Arc::clone(setup.clock), setup.wake.clone()),
        store,
        groups,
        runs: Mutex::new(Vec::new()),
        next_run: AtomicU64::new(0),
        running: Mutex::new(BTreeSet::new()),
    }))
}

/// One guard a run began.
#[derive(Debug, Clone)]
struct RunGuard {
    serial: String,
    handle: u8,
    guard: u64,
    /// Its row once confirmed, cancelled or found ended: answered again from then on.
    settled: Option<Value>,
}

/// A run, for `confirm` and `cancel`.
struct Run {
    id: String,
    owner: Weak<Session>,
    guards: Vec<RunGuard>,
}

impl Run {
    fn owned_by(&self, session: &Arc<Session>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|o| Arc::ptr_eq(&o, session))
    }

    fn gone(&self) -> bool {
        self.owner.upgrade().is_none()
    }

    fn settled(&self) -> bool {
        self.guards.iter().all(|g| g.settled.is_some())
    }
}

struct BulkService {
    exec: Executor,
    store: Arc<HubSnapshots>,
    groups: Arc<dyn GroupResolver>,
    runs: Mutex<Vec<Run>>,
    next_run: AtomicU64,
    /// Sessions with a group request running (`busy` for a second one).
    running: Mutex<BTreeSet<SessionId>>,
}

/// Holds a session's slot in `running` while its request runs.
struct Running<'a> {
    set: &'a Mutex<BTreeSet<SessionId>>,
    id: SessionId,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(self.set).remove(&self.id);
    }
}

impl HubService for BulkService {
    fn name(&self) -> &'static str {
        "bulk"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if !MESSAGES.contains(&t) {
            return None;
        }
        let Some(id) = msg.get("id").filter(|id| id.is_u64() || id.is_i64()) else {
            return Some(super::err(&Value::Null, &ApiError::bad("id is required")));
        };
        let result = require(session, Role::Control).and_then(|()| {
            let _slot = self.enter(session)?;
            match t {
                "group.apply" => self.apply(hub, session, msg),
                "group.apply.confirm" => self.settle(hub, session, msg, Settle::Confirm),
                // "group.apply.cancel", the last of MESSAGES.
                _ => self.settle(hub, session, msg, Settle::Cancel),
            }
        });
        Some(reply(id, result))
    }

    fn next_due(&self) -> Option<Instant> {
        self.exec.next_due()
    }

    fn run_due(&self, hub: &HubHandle, now: Instant) {
        self.exec.sweep(hub, now);
    }

    fn stop(&self) {
        self.exec.close_all();
    }
}

/// What a `confirm` or `cancel` does to each pending guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settle {
    Confirm,
    Cancel,
}

impl BulkService {
    /// `busy` while this session has another group request running.
    fn enter(&self, session: &Session) -> Result<Running<'_>, ApiError> {
        if !lock(&self.running).insert(session.id()) {
            return Err(ApiError::new(
                "busy",
                "this session's other group request is still running",
            ));
        }
        Ok(Running {
            set: &self.running,
            id: session.id(),
        })
    }

    fn apply(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Result<Value, ApiError> {
        let members = addressed(msg, self.groups.as_ref())?;
        let source = self.source(msg)?;
        let guard = match msg.get("guard") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(ApiError::bad("guard is true or false")),
        };
        let timeout_ms = timeout_ms(msg)?;
        let timeout = Duration::from_millis(timeout_ms);
        let run_id = format!(
            "a{}",
            self.next_run
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1)
        );
        let mut rows = Vec::with_capacity(members.len());
        let mut guards: Vec<RunGuard> = Vec::new();
        let mut confirm_by: Option<i64> = None;
        for serial in &members {
            match self.one(hub, session, serial, &source, guard.then_some(timeout)) {
                Ok((row, begun)) => {
                    if let Some((handle, id, started_ms)) = begun {
                        let deadline = started_ms
                            .saturating_add(i64::try_from(timeout_ms).unwrap_or(i64::MAX));
                        confirm_by = Some(confirm_by.map_or(deadline, |c| c.min(deadline)));
                        guards.push(RunGuard {
                            serial: serial.clone(),
                            handle,
                            guard: id,
                            settled: None,
                        });
                    }
                    rows.push(row);
                }
                Err(e) => rows.push(error_row(serial, &e)),
            }
        }
        // The guards that ran out while later members were applied.
        let mut ran_out = 0usize;
        for g in &guards {
            if !pending(hub, g) {
                ran_out = ran_out.saturating_add(1);
                if let Some(row) = rows
                    .iter_mut()
                    .find(|r| r.get("serial").and_then(Value::as_str) == Some(g.serial.as_str()))
                    .and_then(Value::as_object_mut)
                {
                    row.insert("reverted".into(), json!(true));
                }
            }
        }
        let mut body = json!({"run": run_id, "results": rows});
        if let Some(map) = body.as_object_mut() {
            if let Some(ms) = confirm_by {
                let secs = u64::try_from(ms.div_euclid(1000)).unwrap_or(0);
                map.insert("confirm_by".into(), json!(crate::inventory::iso8601(secs)));
            }
            if ran_out > 0 {
                map.insert(
                    "msg".into(),
                    json!(format!(
                        "{ran_out} guard(s) ran out before the run ended and reverted; \
                         raise timeout_ms (at most {MAX_TIMEOUT_MS})"
                    )),
                );
            }
        }
        self.keep(Run {
            id: run_id,
            owner: Arc::downgrade(session),
            guards,
        });
        Ok(body)
    }

    /// One member: its row, and with a guard `(handle, guard id, begun at unix ms)`.
    #[allow(clippy::type_complexity)]
    fn one(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        serial: &str,
        source: &str,
        guard: Option<Duration>,
    ) -> Result<(Value, Option<(u8, u64, i64)>), ApiError> {
        let handle = self.member(hub, session, serial)?;
        if hub
            .apply_status(handle)
            .get("active")
            .is_some_and(|a| !a.is_null())
        {
            return Err(ApiError::new(
                "conflict",
                "a guarded apply is pending on this device; confirm or cancel it first",
            ));
        }
        let begun = match guard {
            Some(timeout) => {
                let started = hub.clock().unix_ms();
                let b = hub
                    .apply_begin(session, handle, timeout)
                    .map_err(|e| refused(&e, "the guard could not begin"))?;
                Some((handle, b.guard, started))
            }
            None => None,
        };
        let text = source.to_owned();
        let done = self.exec.run(hub, session, serial, |cx| {
            let core = cx.core()?;
            let r = core.snapshot_recall_document(text).map_err(|e| match e {
                // The document was checked to be a snapshot: what this device's core
                // refuses now is its platform, wire format or output count.
                dspi_core::DspiError::InvalidArgument { msg } => ApiError::new(
                    "unsupported",
                    format!("this device cannot take the document: {msg}"),
                ),
                other => ApiError::from(other),
            })?;
            let mut record = EventRecord::new(
                EventKind::Recall,
                hub.clock().unix_ms(),
                format!("Recalled snapshot {} on the hub (bulk apply)", r.name),
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
                "ok": true,
                "image": r.image,
                "lines": r.changes,
                "notes": r.notes,
            }))
        });
        match (done, begun) {
            (Ok(mut row), Some((handle, id, started))) => {
                if let Some(m) = row.as_object_mut() {
                    m.insert("guard".into(), json!(id));
                }
                Ok((row, Some((handle, id, started))))
            }
            (Ok(row), None) => Ok((row, None)),
            (Err(e), Some(_)) => {
                // Back to what the guard captured, now, rather than at its deadline.
                if let Err(c) = hub.apply_cancel(session, handle) {
                    log::warn!(
                        "hub bulk apply: a failed recall's guard did not revert: {}",
                        c.code()
                    );
                }
                Err(e)
            }
            (Err(e), None) => Err(e),
        }
    }

    /// The handle of the online member `serial`: `offline` when it is known (the
    /// registry or the inventory) and not attached, else `not_found`.
    fn member(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        serial: &str,
    ) -> Result<u8, ApiError> {
        let devices = hub.devices();
        if let Some(d) = devices.iter().find(|d| d.identity.serial == serial) {
            return if d.state == DeviceState::Online {
                Ok(d.handle)
            } else {
                Err(ApiError::offline())
            };
        }
        let ask = json!({"t": "inventory.list", "serial": serial});
        let known = hub
            .service_request(session, &ask)
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|v| v.get("t").and_then(Value::as_str) == Some("ok"));
        if known {
            Err(ApiError::offline())
        } else {
            Err(ApiError::new(
                "not_found",
                format!("no device {serial} is known"),
            ))
        }
    }

    /// The document a request names, as text.
    fn source(&self, msg: &Value) -> Result<String, ApiError> {
        let from_store =
            |e: crate::snapshots::SnapshotError| ApiError::new(e.code(), e.to_string());
        let given = ["snapshot", "name", "document"]
            .iter()
            .filter(|k| msg.get(**k).is_some_and(|v| !v.is_null()))
            .count();
        if given != 1 {
            return Err(ApiError::bad(
                "exactly one of snapshot, name and document is required",
            ));
        }
        let id = match (msg.get("snapshot"), msg.get("name")) {
            (Some(Value::String(id)), _) => Some(id.clone()),
            (Some(v), _) if !v.is_null() => {
                return Err(ApiError::bad("snapshot is a snapshot's id"));
            }
            (_, Some(Value::String(name))) => Some(self.by_name(name)?),
            (_, Some(v)) if !v.is_null() => return Err(ApiError::bad("name is a snapshot's name")),
            _ => None,
        };
        let text = match id {
            Some(id) => self.store.get(&id).map_err(from_store)?.1,
            None => document(msg.get("document"))?,
        };
        check_snapshot(&text)?;
        Ok(text)
    }

    /// The one snapshot of the whole store named `name`.
    fn by_name(&self, name: &str) -> Result<String, ApiError> {
        let from_store =
            |e: crate::snapshots::SnapshotError| ApiError::new(e.code(), e.to_string());
        let mut found = Vec::new();
        for serial in store_serials(&self.store) {
            for e in self.store.list(&serial).map_err(from_store)? {
                if e.name == name {
                    found.push(e.id);
                }
            }
        }
        match found.as_slice() {
            [id] => Ok(id.clone()),
            [] => Err(ApiError::new(
                "not_found",
                format!("no snapshot is named {name}"),
            )),
            _ => Err(ApiError::new(
                "conflict",
                format!(
                    "{} snapshots are named {name}; apply one by id",
                    found.len()
                ),
            )),
        }
    }

    /// Keep a run, dropping first those whose session is gone, then the oldest settled
    /// one, then the oldest.
    fn keep(&self, run: Run) {
        let mut runs = lock(&self.runs);
        runs.retain(|r| !r.gone());
        if runs.len() >= MAX_RUNS {
            let at = runs.iter().position(Run::settled).unwrap_or(0);
            if at < runs.len() {
                runs.remove(at);
            }
        }
        runs.push(run);
    }

    /// `group.apply.confirm` or `group.apply.cancel`.
    fn settle(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
        how: Settle,
    ) -> Result<Value, ApiError> {
        let run_id = match msg.get("run") {
            Some(Value::String(r)) if !r.is_empty() && r.len() <= 24 => r.clone(),
            _ => return Err(ApiError::bad("run is a bulk apply's run")),
        };
        let not_found = || ApiError::new("not_found", format!("no run {run_id} of this session"));
        let guards = {
            let runs = lock(&self.runs);
            runs.iter()
                .find(|r| r.id == run_id && r.owned_by(session))
                .map(|r| r.guards.clone())
                .ok_or_else(not_found)?
        };
        let mut rows = Vec::with_capacity(guards.len());
        let mut settled = Vec::with_capacity(guards.len());
        for g in &guards {
            let row = match &g.settled {
                Some(row) => row.clone(),
                None => settle_one(hub, session, g, how),
            };
            settled.push(row.clone());
            rows.push(row);
        }
        {
            let mut runs = lock(&self.runs);
            if let Some(r) = runs.iter_mut().find(|r| r.id == run_id) {
                for (g, row) in r.guards.iter_mut().zip(settled) {
                    g.settled.get_or_insert(row);
                }
            }
        }
        Ok(json!({"run": run_id, "results": rows}))
    }
}

/// Whether the run's guard `g` is still the device's pending one.
fn pending(hub: &HubHandle, g: &RunGuard) -> bool {
    let status = hub.apply_status(g.handle);
    let active = status.get("active");
    active.and_then(|a| a.get("guard")).and_then(Value::as_u64) == Some(g.guard)
        && !active
            .and_then(|a| a.get("reverting"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Confirm or cancel one pending guard: its row.
fn settle_one(hub: &HubHandle, session: &Session, g: &RunGuard, how: Settle) -> Value {
    if !pending(hub, g) {
        let status = hub.apply_status(g.handle);
        let last = status
            .get("last")
            .filter(|l| l.get("guard").and_then(Value::as_u64) == Some(g.guard));
        let how_it_ended = last
            .and_then(|l| {
                Some(format!(
                    "{}, {}",
                    l.get("reason")?.as_str()?,
                    l.get("result")?.as_str()?
                ))
            })
            .unwrap_or_else(|| "it is being reverted".into());
        return json!({"serial": g.serial, "ok": false, "code": "conflict", "guard": g.guard,
            "msg": format!("the guarded apply had already ended ({how_it_ended})")});
    }
    let result = match how {
        Settle::Confirm => hub.apply_confirm(session, g.handle),
        Settle::Cancel => hub.apply_cancel(session, g.handle),
    };
    match result {
        Ok(o) => match (how, o.result) {
            (Settle::Confirm, _) | (Settle::Cancel, EndResult::Reverted) => {
                let mut row = json!({"serial": g.serial, "ok": true, "guard": g.guard});
                if how == Settle::Cancel
                    && let Some(m) = row.as_object_mut()
                {
                    m.insert("reverted".into(), json!(true));
                }
                row
            }
            (Settle::Cancel, EndResult::NotWritten) => {
                error_row(&g.serial, &ApiError::offline()).with_guard(g.guard)
            }
            (Settle::Cancel, other) => {
                let mut e =
                    ApiError::new("device", format!("the revert was not written ({other:?})"));
                if let Some(s) = o.status {
                    e = e.with("status", json!(s));
                }
                error_row(&g.serial, &e).with_guard(g.guard)
            }
        },
        Err(e) => error_row(&g.serial, &refused(&e, "the guarded apply")).with_guard(g.guard),
    }
}

trait WithGuard {
    fn with_guard(self, guard: u64) -> Value;
}

impl WithGuard for Value {
    fn with_guard(mut self, guard: u64) -> Value {
        if let Some(m) = self.as_object_mut() {
            m.insert("guard".into(), json!(guard));
        }
        self
    }
}

/// A guard's refusal as this API's error: `no_device` is `offline` here (section 19.1's
/// codes), an ended guard `conflict`.
fn refused(e: &ApplyRefused, what: &str) -> ApiError {
    let code = match e {
        ApplyRefused::NoDevice => "offline",
        other => other.code(),
    };
    ApiError::new(code, format!("{what}: {e:?}"))
}

/// A failed row: `serial`, `ok: false`, the code, its line and its fields.
fn error_row(serial: &str, e: &ApiError) -> Value {
    let mut row = Map::new();
    row.insert("serial".into(), json!(serial));
    row.insert("ok".into(), json!(false));
    row.insert("code".into(), json!(e.code));
    row.insert("msg".into(), json!(e.msg));
    for (k, v) in &e.extra {
        row.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Value::Object(row)
}

/// The devices a request names: `group` (its members, in member order) or `targets`
/// (1 to 16 serials, each once). Exactly one; more than [`MAX_RESULT_ROWS`] is
/// `bad_request` with nothing sent.
fn addressed(msg: &Value, groups: &dyn GroupResolver) -> Result<Vec<String>, ApiError> {
    let group = msg.get("group").filter(|v| !v.is_null());
    let targets = msg.get("targets").filter(|v| !v.is_null());
    let members = match (group, targets) {
        (Some(Value::String(id)), None) => groups
            .members(id)
            .ok_or_else(|| ApiError::new("not_found", format!("no group {id}")))?,
        (Some(_), None) => return Err(ApiError::bad("group is a group's id")),
        (None, Some(Value::Array(list))) => {
            if list.is_empty() || list.len() > MAX_TARGETS {
                return Err(ApiError::bad(format!(
                    "targets is 1 to {MAX_TARGETS} serials"
                )));
            }
            let mut seen = BTreeSet::new();
            let mut out = Vec::with_capacity(list.len());
            for t in list {
                let s = t
                    .as_str()
                    .filter(|s| crate::inventory::valid_serial(s))
                    .ok_or_else(|| ApiError::bad("targets are devices' serials"))?;
                if !seen.insert(s) {
                    return Err(ApiError::bad(format!("{s} is in targets twice")));
                }
                out.push(s.to_owned());
            }
            out
        }
        (None, Some(_)) => return Err(ApiError::bad("targets is a list of serials")),
        (Some(_), Some(_)) => return Err(ApiError::bad("group or targets, not both")),
        (None, None) => return Err(ApiError::bad("group or targets is required")),
    };
    if members.len() > MAX_RESULT_ROWS {
        return Err(ApiError::bad(format!(
            "a run answers at most {MAX_RESULT_ROWS} rows; this one would answer {}",
            members.len()
        )));
    }
    Ok(members)
}

/// `timeout_ms`: 1000 to 600000, 30000 when absent.
fn timeout_ms(msg: &Value) -> Result<u64, ApiError> {
    match msg.get("timeout_ms") {
        None | Some(Value::Null) => Ok(DEFAULT_TIMEOUT_MS),
        Some(v) => v
            .as_u64()
            .filter(|ms| (MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(ms))
            .ok_or_else(|| {
                ApiError::bad(format!(
                    "timeout_ms is {MIN_TIMEOUT_MS} to {MAX_TIMEOUT_MS}"
                ))
            }),
    }
}

/// `document`: its text or the JSON object, compact, at most one frame's worth.
fn document(v: Option<&Value>) -> Result<String, ApiError> {
    let text = match v {
        Some(Value::String(s)) => s.clone(),
        Some(o @ Value::Object(_)) => o.to_string(),
        _ => return Err(ApiError::bad("document is a snapshot's text or its object")),
    };
    let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if len > MAX_WIRE_DOC_BYTES {
        return Err(ApiError::bad(format!(
            "the document is {len} bytes; one frame carries at most {MAX_WIRE_DOC_BYTES}"
        )));
    }
    Ok(text)
}

/// The request as a whole is `bad_request` when its document is not a snapshot, so a
/// row's refusal is the device's (`unsupported`), not the document's.
fn check_snapshot(text: &str) -> Result<(), ApiError> {
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| ApiError::bad("the document is not JSON"))?;
    let kind = doc
        .get("meta")
        .and_then(|m| m.get("kind"))
        .and_then(Value::as_str);
    if kind == Some("snapshot") && doc.get("snapshot").is_some_and(Value::is_object) {
        Ok(())
    } else {
        Err(ApiError::bad(
            "the document is not a snapshot (meta.kind \"snapshot\" with its snapshot block)",
        ))
    }
}

/// Every serial the store keeps a folder for.
fn store_serials(store: &HubSnapshots) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(store.root()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            dspi_session::snapshot_store::serial_of_dir(&e.file_name().into_string().ok()?)
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p3ba {
    use super::*;

    struct Fake(usize);

    impl GroupResolver for Fake {
        fn members(&self, id: &str) -> Option<Vec<String>> {
            (id == "big").then(|| (0..self.0).map(|i| format!("S{i:04}")).collect())
        }
    }

    #[test]
    fn wp_p3ba_a_run_past_256_rows_is_bad_request() {
        let e = addressed(&json!({"group": "big"}), &Fake(300)).unwrap_err();
        assert_eq!(e.code, "bad_request");
        assert!(e.msg.contains("256"), "{}", e.msg);
        assert_eq!(
            addressed(&json!({"group": "big"}), &Fake(256))
                .unwrap()
                .len(),
            256
        );
    }

    #[test]
    fn wp_p3ba_addressing_is_a_group_or_1_to_16_targets_each_once() {
        let f = Fake(2);
        assert_eq!(
            addressed(&json!({"group": "nope"}), &f).unwrap_err().code,
            "not_found"
        );
        let code = |m: Value| addressed(&m, &f).unwrap_err().code;
        assert_eq!(code(json!({})), "bad_request");
        assert_eq!(
            code(json!({"group": "big", "targets": ["A"]})),
            "bad_request"
        );
        assert_eq!(code(json!({"targets": []})), "bad_request");
        assert_eq!(code(json!({"targets": ["A", "A"]})), "bad_request");
        let many: Vec<String> = (0..17).map(|i| format!("T{i}")).collect();
        assert_eq!(code(json!({"targets": many})), "bad_request");
        assert_eq!(
            addressed(&json!({"targets": ["B", "A"]}), &f).unwrap(),
            vec!["B".to_owned(), "A".to_owned()]
        );
    }

    #[test]
    fn wp_p3ba_timeout_is_1_to_600_seconds() {
        assert_eq!(timeout_ms(&json!({})).unwrap(), DEFAULT_TIMEOUT_MS);
        assert_eq!(timeout_ms(&json!({"timeout_ms": 1000})).unwrap(), 1000);
        assert!(timeout_ms(&json!({"timeout_ms": 999})).is_err());
        assert!(timeout_ms(&json!({"timeout_ms": 600_001})).is_err());
        assert!(timeout_ms(&json!({"timeout_ms": "5"})).is_err());
    }

    #[test]
    fn wp_p3ba_a_document_must_be_a_snapshot_within_a_frame() {
        let snap = json!({"meta": {"kind": "snapshot"}, "snapshot": {}});
        assert!(check_snapshot(&document(Some(&snap)).unwrap()).is_ok());
        assert!(check_snapshot(&document(Some(&json!(snap.to_string()))).unwrap()).is_ok());
        let preset = json!({"meta": {"kind": "preset"}});
        assert_eq!(
            check_snapshot(&preset.to_string()).unwrap_err().code,
            "bad_request"
        );
        assert_eq!(check_snapshot("{").unwrap_err().code, "bad_request");
        assert_eq!(document(Some(&json!(5))).unwrap_err().code, "bad_request");
        let big = "x".repeat(usize::try_from(MAX_WIRE_DOC_BYTES).unwrap() + 1);
        assert_eq!(document(Some(&json!(big))).unwrap_err().code, "bad_request");
    }
}
