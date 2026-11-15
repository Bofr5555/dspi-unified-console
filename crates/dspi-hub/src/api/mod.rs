//! The hub's parameter API, its action executor and hub-side snapshot recall (WP P2PA,
//! PLAN-P2 I1, I2's preset half, OD-P1b's recall; `docs/integrator/hub-api.md` sections
//! 9, 10 and 11.2), and the registration of the hub's snapshot store (P2SS) as the
//! `snapshot` family.
//!
//! | Module | What |
//! |---|---|
//! | [`transport`] | a `Transport` that submits as a principal's child session |
//! | `exec` (feature `api`) | one `DspiCore` per (principal, device): the action executor |
//! | `params` (feature `api`) | `param.list`, `param.get`, `param.get_many`, `param.set` |
//! | `presets` (feature `api`) | `preset.list`, `preset.load`, `preset.save` |
//! | `recall` (feature `api`) | `snapshot.recall` from the hub's store |
//!
//! **Every write is judged.** A write runs through `dspi-core`'s `write_param` (or the
//! typed call: `load_preset`, `snapshot_recall_document`) inside the hub, on a core
//! opened over a **child session** of the caller: its role, its client, its own session
//! id from the remote range (the session-id rule, PLAN-P2 M2; never `LOCAL_SESSION_ID`).
//! So every typed setter's guard, `write_refusal`, the limits, the installer lock, the
//! device lock and the voltage rule are those a console meets, and other sessions see
//! the write as a foreign one, attributed to the caller's name. Nothing here reaches a
//! raw opcode: `param.set` takes registry paths only, refuses raw bytes and the
//! extension opcode, and never clears the clip latch (`meters.clear`).
//!
//! **Gated paths** (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`) are
//! written only for an Admin whose request carries `"confirm": true`, the API's explicit
//! confirmation: the hub's core then mints and spends the token itself (P1TK's
//! discipline). **A pending guarded apply** of another session on the device refuses an
//! action `conflict` (review S4). Answers carry `"v": 1`; the cap is [`CAP`].

use std::sync::Arc;

use serde_json::Value;
#[cfg(feature = "api")]
use serde_json::{Map, json};

use crate::hub::HubHandle;
#[cfg(feature = "api")]
use crate::registry::DeviceState;
use crate::service::{HubService, ServiceSetup};
use crate::session::Session;
use crate::snapshots::HubSnapshots;

pub mod transport;

#[cfg(feature = "api")]
mod exec;
#[cfg(feature = "api")]
mod params;
#[cfg(feature = "api")]
mod presets;
#[cfg(feature = "api")]
mod recall;

#[cfg(feature = "api")]
pub use exec::{IDLE_CLOSE, MAX_CORES, SWEEP};

/// The `hello` cap of the parameter API (sections 9 and 10).
pub const CAP: &str = "param_api";
/// The messages' version, `v` in every answer.
pub const VERSION: u32 = 1;

/// The services of this module, in the order they are consulted: the `snapshot` family
/// (when the hub has a data folder), then the parameter API (with feature `api`).
pub fn services(setup: &ServiceSetup<'_>) -> Vec<Arc<dyn HubService>> {
    let store = setup.data_dir.map(|dir| Arc::new(HubSnapshots::open(dir)));
    let mut list: Vec<Arc<dyn HubService>> = Vec::new();
    if let Some(store) = &store {
        list.push(Arc::new(SnapshotService {
            store: Arc::clone(store),
        }));
    }
    #[cfg(feature = "api")]
    list.push(Arc::new(ApiService::new(setup, store)));
    list
}

// ── The snapshot family (P2SS's store and handlers) ─────────────────────────

/// `snapshot.list`, `get`, `put`, `rename`, `note`, `delete`: P2SS's handlers, with each
/// change broadcast to every session as `snapshot.changed` after the answer is made.
struct SnapshotService {
    store: Arc<HubSnapshots>,
}

impl HubService for SnapshotService {
    fn name(&self) -> &'static str {
        "snapshots"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![crate::snapshots::CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let answer = self.store.handle_message(session.role(), msg)?;
        for event in self.store.take_changed(hub.hub_id()) {
            hub.inner.relay.broadcast_event(&event);
        }
        Some(answer)
    }
}

// ── Answers and errors ──────────────────────────────────────────────────────

#[cfg(feature = "api")]
/// A refusal of this API: a code of `hub-api.md` section 1.6, a line for people, and
/// any more fields its row names (`min`, `max`, `limit`, `use`, `status`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApiError {
    pub(crate) code: &'static str,
    pub(crate) msg: String,
    pub(crate) extra: Map<String, Value>,
}

#[cfg(feature = "api")]
impl ApiError {
    pub(crate) fn new(code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
            extra: Map::new(),
        }
    }

    pub(crate) fn bad(msg: impl Into<String>) -> Self {
        Self::new("bad_request", msg)
    }

    pub(crate) fn offline() -> Self {
        Self::new("offline", "the device is not attached; nothing was sent")
    }

    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self::new("internal", msg)
    }

    pub(crate) fn with(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.to_owned(), value);
        self
    }

    /// A transport's failure, as this API's code.
    pub(crate) fn transport(e: dspi_transport::TransportError) -> Self {
        use dspi_transport::TransportError as T;
        if let Some((key, msg)) = e.hub_refusal() {
            let code = match key {
                "limit_exceeded" => "limit_exceeded",
                "installer_lock" => "installer_lock",
                "image_refused" => "image_refused",
                _ => "busy",
            };
            let mut err = Self::new(code, msg);
            if let Some(limit) = e.hub_refusal_limit() {
                err = err.with("limit", json!(limit));
            }
            return err;
        }
        match e {
            T::Denied(m) => Self::new("denied", m),
            T::Locked => Self::new("locked", "another session holds the device's lock"),
            T::Disconnected | T::NotFound | T::SerialNotFound(_) => Self::offline(),
            T::Stalled { opcode } => Self::new(
                "device",
                format!("the device refused opcode 0x{opcode:02X}"),
            )
            .with("status", json!("ERROR")),
            other => Self::internal(other.to_string()),
        }
    }
}

#[cfg(feature = "api")]
impl From<dspi_core::DspiError> for ApiError {
    fn from(e: dspi_core::DspiError) -> Self {
        use dspi_core::DspiError as E;
        let msg = e.to_string();
        match e {
            E::Disconnected => Self::offline(),
            E::InvalidArgument { msg } => Self::bad(msg),
            E::Unsupported => Self::new("unsupported", msg),
            E::ReadOnly => Self::new(
                "unsupported",
                "this firmware's wire format is read-only for this hub",
            ),
            E::Locked => Self::new("locked", "another session holds the device's lock"),
            E::Denied => Self::new("denied", "this role may not do that on the device"),
            E::Refused { status } => Self::new("device", msg).with("status", json!(status)),
            E::ConfirmationRequired { .. } => Self::new("confirmation_required", msg),
            E::LimitExceeded { code, .. } => {
                Self::new("limit_exceeded", msg).with("limit", json!(code))
            }
            E::InstallerLock { .. } => Self::new("installer_lock", msg),
            E::ImageRefused { .. } => Self::new("image_refused", msg),
            E::LimitsUnknown { .. } => Self::new("busy", msg),
            E::OvervoltWordRequired { .. } | E::FirmwareGuardBlocked { .. } => {
                Self::new("denied", msg)
            }
            E::Transfer { .. } | E::Link { .. } | E::Io { .. } | E::AutoEq { .. } => {
                Self::internal(msg)
            }
        }
    }
}

#[cfg(feature = "api")]
/// `{"t": "ok", "id", "v": 1, ...body}`.
pub(crate) fn ok(id: &Value, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

#[cfg(feature = "api")]
/// `{"t": "err", "id", "v": 1, "code", "msg", ...extra}`.
pub(crate) fn err(id: &Value, e: &ApiError) -> String {
    let mut out = json!({"t": "err", "id": id, "v": VERSION, "code": e.code, "msg": e.msg});
    if let Some(map) = out.as_object_mut() {
        map.extend(e.extra.clone());
    }
    out.to_string()
}

#[cfg(feature = "api")]
/// The answer to a request: `ok` with the body, or `err`.
pub(crate) fn reply(id: &Value, result: Result<Value, ApiError>) -> String {
    match result {
        Ok(body) => ok(id, body),
        Err(e) => err(id, &e),
    }
}

#[cfg(feature = "api")]
/// The device a request addresses (`hub-api.md` 1.3): by `serial`, or by `handle` for
/// an online one; both must name the same device. A serial the hub does not know is
/// `not_found`, one it knows but not attached is `offline`, an unknown handle
/// `no_device`. Answers the serial and the handle of the online device.
pub(crate) fn target(hub: &HubHandle, msg: &Value) -> Result<(String, u8), ApiError> {
    let serial = match msg.get("serial") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 64 => Some(s.as_str()),
        Some(_) => return Err(ApiError::bad("serial must be a device's serial")),
    };
    let handle = match msg.get("handle") {
        None | Some(Value::Null) => None,
        Some(h) => Some(
            h.as_u64()
                .and_then(|h| u8::try_from(h).ok())
                .ok_or_else(|| ApiError::bad("handle must be a whole number 0 to 255"))?,
        ),
    };
    let devices = hub.devices();
    match (serial, handle) {
        (None, None) => Err(ApiError::bad("serial is required")),
        (Some(serial), _) => {
            let d = devices
                .iter()
                .find(|d| d.identity.serial == serial)
                .ok_or_else(|| {
                    ApiError::new("not_found", format!("no device {serial} is known"))
                })?;
            if handle.is_some_and(|h| h != d.handle || d.state != DeviceState::Online) {
                return Err(ApiError::bad("serial and handle name different devices"));
            }
            if d.state != DeviceState::Online {
                return Err(ApiError::offline());
            }
            Ok((serial.to_owned(), d.handle))
        }
        (None, Some(h)) => devices
            .iter()
            .find(|d| d.handle == h && d.state == DeviceState::Online)
            .map(|d| (d.identity.serial.clone(), h))
            .ok_or_else(|| ApiError::new("no_device", format!("no device at handle {h}"))),
    }
}

// ── The parameter API's service ─────────────────────────────────────────────

/// `param.*`, `preset.*` and `snapshot.recall`, over the action executor.
#[cfg(feature = "api")]
pub(crate) struct ApiService {
    exec: exec::Executor,
    store: Option<Arc<HubSnapshots>>,
}

#[cfg(feature = "api")]
impl ApiService {
    fn new(setup: &ServiceSetup<'_>, store: Option<Arc<HubSnapshots>>) -> Self {
        Self {
            exec: exec::Executor::new(Arc::clone(setup.clock), setup.wake.clone()),
            store,
        }
    }
}

/// The messages of this family.
pub const MESSAGES: [&str; 8] = [
    "param.list",
    "param.get",
    "param.get_many",
    "param.set",
    "preset.list",
    "preset.load",
    "preset.save",
    "snapshot.recall",
];

#[cfg(feature = "api")]
impl HubService for ApiService {
    fn name(&self) -> &'static str {
        "api"
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
            return Some(err(&Value::Null, &ApiError::bad("id is required")));
        };
        let result = match t {
            "param.list" | "param.get" | "param.get_many" | "param.set" => {
                params::handle(&self.exec, hub, session, t, msg)
            }
            "preset.list" | "preset.load" | "preset.save" => {
                presets::handle(&self.exec, hub, session, t, msg)
            }
            // "snapshot.recall", the last of MESSAGES.
            _ => recall::handle(&self.exec, self.store.as_deref(), hub, session, msg),
        };
        Some(reply(id, result))
    }

    fn next_due(&self) -> Option<std::time::Instant> {
        self.exec.next_due()
    }

    fn run_due(&self, hub: &HubHandle, now: std::time::Instant) {
        self.exec.sweep(hub, now);
    }

    fn stop(&self) {
        self.exec.close_all();
    }
}

// ── Shared checks of the actions ────────────────────────────────────────────

#[cfg(feature = "api")]
/// `denied` below `need`.
pub(crate) fn require(session: &Session, need: crate::session::Role) -> Result<(), ApiError> {
    if session.role() < need {
        return Err(ApiError::new("denied", "this role may not do that"));
    }
    Ok(())
}

#[cfg(feature = "api")]
/// `conflict` while another session's guarded apply is pending on the device at
/// `handle` (review S4): a guard of `principal` itself (a guarded recall) is its own.
pub(crate) fn no_foreign_guard(
    hub: &HubHandle,
    handle: u8,
    principal: crate::session::SessionId,
) -> Result<(), ApiError> {
    let status = hub.apply_status(handle);
    let owner = status
        .get("active")
        .and_then(|a| a.get("session"))
        .and_then(Value::as_u64);
    match owner {
        Some(owner) if owner != u64::from(principal) => Err(ApiError::new(
            "conflict",
            "a guarded apply is pending on this device; confirm or cancel it first",
        )),
        _ => Ok(()),
    }
}

#[cfg(feature = "api")]
#[cfg(test)]
mod tests_wp_p2pa {
    use super::*;

    #[test]
    fn wp_p2pa_errors_carry_v_code_msg_and_their_fields() {
        let e = ApiError::bad("out of range")
            .with("min", json!(-127.0))
            .with("max", json!(0.0));
        let v: Value = serde_json::from_str(&err(&json!(4), &e)).unwrap();
        assert_eq!(v["t"], "err");
        assert_eq!(v["id"], 4);
        assert_eq!(v["v"], 1);
        assert_eq!(v["code"], "bad_request");
        assert_eq!(v["max"], 0.0);
        let v: Value = serde_json::from_str(&ok(&json!(5), json!({"value": 1}))).unwrap();
        assert_eq!(
            (v["t"].as_str(), v["v"].as_u64(), v["value"].as_u64()),
            (Some("ok"), Some(1), Some(1))
        );
    }

    #[test]
    fn wp_p2pa_a_hub_refusal_is_its_code_with_its_limit() {
        let e = ApiError::transport(dspi_transport::TransportError::Denied(
            "limit_exceeded/out_gain: over the limit".into(),
        ));
        assert_eq!(e.code, "limit_exceeded");
        assert_eq!(e.extra["limit"], "out_gain");
        let e = ApiError::transport(dspi_transport::TransportError::Denied("nope".into()));
        assert_eq!(e.code, "denied");
    }

    #[cfg(feature = "api")]
    #[test]
    fn wp_p2pa_core_errors_map_to_the_api_codes() {
        use dspi_core::DspiError as E;
        let code = |e: E| ApiError::from(e).code;
        assert_eq!(code(E::Disconnected), "offline");
        assert_eq!(code(E::Locked), "locked");
        assert_eq!(code(E::Denied), "denied");
        assert_eq!(code(E::ImageRefused { msg: "x".into() }), "image_refused");
        assert_eq!(
            code(E::InstallerLock {
                code: "hub".into(),
                msg: "x".into()
            }),
            "installer_lock"
        );
        assert_eq!(
            code(E::ConfirmationRequired {
                code: "missing".into(),
                msg: "x".into()
            }),
            "confirmation_required"
        );
        let e = ApiError::from(E::LimitExceeded {
            code: "out_gain".into(),
            msg: "x".into(),
        });
        assert_eq!(
            (e.code, e.extra["limit"].as_str()),
            ("limit_exceeded", Some("out_gain"))
        );
    }
}
