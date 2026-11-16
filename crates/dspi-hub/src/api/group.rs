//! Linked parameters (WP P3GP, PLAN-P3 F4, OD-P3c; `hub-api.md` section 20.1 and 20.2):
//! one-shot fan-outs over a group or a list of serials.
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `group.param.get {group or targets, path, idx?}` | viewer | `path`, `idx`, `results` with `value` |
//! | `group.param.set {group or targets, path, idx?, value, offsets?}` | control | `path`, `idx`, `results` with `value`, `clamped` |
//! | `group.preset.load {group or targets, slot or name, map?}` | control | `results` with `slot`, `name` |
//!
//! **Each member is the single-device message.** A row is `param.get`, `param.set` or
//! `preset.load` of sections 9 and 10 run through this family's executor as the caller's
//! child session ([`super::params`], [`super::presets`]): the same paths, values and
//! guards, so each member is judged on its own by role, `write_refusal`, its limits
//! profile, the installer lock, the device lock and the voltage rule. One member's
//! refusal never stops the others; nothing is retried and no link is kept (OD-P3c).
//! Never a raw opcode: the paths are the API's.
//!
//! **Sequential and bounded:** members in group order, one after another; `group`
//! through P3GR's [`GroupResolver`] (at most 64 members), or `targets` (1 to
//! [`MAX_TARGETS`] serials); at most [`MAX_RESULT_ROWS`] rows. One group request at a
//! time per session (`busy` otherwise).
//!
//! **Offsets** are taken only on a path whose unit is dB: a member gets `value` plus its
//! offset, clamped into that device's own range (`Session::write_range`: the registry's,
//! narrowed by its platform), and the row says `clamped: true` when the clamp moved it.

use std::sync::Arc;
use std::time::Instant;

use dspi_proto::registry::{Kind, ParamDesc, by_path};
use dspi_proto::value::Unit;
use serde_json::{Map, Value, json};

use super::exec::Executor;
use super::{ApiError, GroupBusy, err, ok, require};
use crate::groups::{GroupResolver, MAX_RESULT_ROWS, valid_group_target};
use crate::hub::HubHandle;
use crate::inventory::valid_serial;
use crate::registry::DeviceState;
use crate::service::{HubService, ServiceSetup};
use crate::session::{Role, Session};

/// The `hello` cap of this family (`hub-api.md` section 20).
pub const CAP: &str = "group_params";
/// The messages of this family.
pub const MESSAGES: [&str; 3] = ["group.param.get", "group.param.set", "group.preset.load"];
/// The most serials one `targets` list names.
pub const MAX_TARGETS: usize = 16;
/// Preset slots on every DSPi.
const SLOTS: u64 = 10;
/// A preset name's most, in bytes (the firmware's 32 with its NUL).
const NAME_BYTES: usize = 31;
/// The longest group id taken, in bytes (as `groups.rs`).
const MAX_GROUP_ID_BYTES: usize = 24;

/// The service for `api::services`. -- WP P3RS -- `busy` is shared with bulk apply.
pub(super) fn service(setup: &ServiceSetup<'_>, busy: GroupBusy) -> Arc<dyn HubService> {
    Arc::new(GroupParams {
        exec: Executor::new(Arc::clone(setup.clock), setup.wake.clone()),
        groups: setup.group_resolver(),
        running: busy,
    })
}

/// `group.param.*` and `group.preset.load`, over an executor of their own.
struct GroupParams {
    exec: Executor,
    groups: Option<Arc<dyn GroupResolver>>,
    // -- WP P3RS --
    /// The sessions with a group request running, shared with `group.apply*`.
    running: GroupBusy,
}

impl HubService for GroupParams {
    fn name(&self) -> &'static str {
        "group_params"
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
        Some(match self.handle(hub, session, t, msg) {
            Ok(body) => ok(id, body),
            Err(e) => err(id, &e),
        })
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

// ── The request ─────────────────────────────────────────────────────────────

/// How the request names its devices.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Address {
    /// A group's id.
    Group(String),
    /// Serials, each once.
    Targets(Vec<String>),
}

/// `group` or `targets`, exactly one (`hub-api.md` 20.1).
fn address(msg: &Value) -> Result<Address, ApiError> {
    let group = msg.get("group").filter(|v| !v.is_null());
    let targets = msg.get("targets").filter(|v| !v.is_null());
    match (group, targets) {
        (Some(_), Some(_)) => Err(ApiError::bad(
            "name the devices by group or by targets, not both",
        )),
        (None, None) => Err(ApiError::bad("group or targets is required")),
        (Some(g), None) => {
            let id = g
                .as_str()
                .filter(|id| id.len() <= MAX_GROUP_ID_BYTES)
                .filter(|id| valid_group_target(&format!("group:{id}")))
                .ok_or_else(|| ApiError::bad("group is a group's id"))?;
            Ok(Address::Group(id.to_owned()))
        }
        (None, Some(t)) => {
            let list = t
                .as_array()
                .ok_or_else(|| ApiError::bad("targets is a list of serials"))?;
            if list.is_empty() || list.len() > MAX_TARGETS {
                return Err(ApiError::bad(format!(
                    "targets names 1 to {MAX_TARGETS} serials"
                )));
            }
            let mut out: Vec<String> = Vec::with_capacity(list.len());
            for s in list {
                let s = s
                    .as_str()
                    .filter(|s| valid_serial(s))
                    .ok_or_else(|| ApiError::bad("targets is a list of serials"))?;
                if s.starts_with(crate::groups::TARGET_PREFIX) {
                    return Err(ApiError::bad(
                        "targets are serials; name a group with group",
                    ));
                }
                if out.iter().any(|x| x == s) {
                    return Err(ApiError::bad(format!("{s} is named twice in targets")));
                }
                out.push(s.to_owned());
            }
            Ok(Address::Targets(out))
        }
    }
}

/// The request's `path` and `idx`, as `param.*` takes them.
fn path_and_idx(msg: &Value) -> Result<(&'static ParamDesc, Value), ApiError> {
    let path = msg
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("path is required"))?;
    let d = by_path(path).ok_or_else(|| ApiError::bad(format!("there is no parameter {path}")))?;
    let idx = match msg.get("idx") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|i| i.as_u64().and_then(|i| u8::try_from(i).ok()))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| ApiError::bad("idx holds whole numbers 0 to 255"))?,
        Some(_) => return Err(ApiError::bad("idx is an array of whole numbers")),
    };
    let want = d.target.arity();
    if idx.len() != want {
        return Err(ApiError::bad(format!(
            "{path} takes {want} index(es) in idx, not {}",
            idx.len()
        )));
    }
    Ok((d, json!(idx)))
}

/// Whether offsets may be given on `d` and its values are clamped: a dB path.
fn is_db(d: &ParamDesc) -> bool {
    matches!(d.kind, Kind::Float { unit: Unit::Db, .. })
}

/// The request's `value` for `d`: of its kind and, for a number, inside the registry's
/// range (the device's own, narrower one is each row's). A gated path is not taken.
fn check_value(d: &ParamDesc, v: Option<&Value>) -> Result<Value, ApiError> {
    if let Some(why) = super::params::api_refusal(d) {
        return Err(ApiError::bad(why));
    }
    if dspi_core::confirm::gated_path(d.path).is_some() {
        return Err(ApiError::bad(format!(
            "{} is written with confirmation: use param.set on one device",
            d.path
        )));
    }
    let v = v.cloned().unwrap_or(Value::Null);
    let wrong = |what: &str| ApiError::bad(format!("{} ({}) takes {what}", d.label, d.path));
    let fits = match d.kind {
        Kind::Trigger => v.is_null(),
        Kind::Bool => v.is_boolean(),
        Kind::Choice(_) => v.is_string() || v.as_u64().is_some_and(|n| n <= 255),
        Kind::Mask => v.as_u64().is_some_and(|m| m <= u64::from(u32::MAX)),
        Kind::Text { .. } => v.is_string(),
        Kind::Float { min, max, .. } => {
            let f = v
                .as_f64()
                .filter(|f| f.is_finite())
                .ok_or_else(|| wrong("a number"))?;
            // A delay's range is the device's (CG37): each row's to check.
            let delay = matches!(d.path, "out.delay" | "ch.delay");
            if !delay && !(f64::from(min)..=f64::from(max)).contains(&f) {
                return Err(range_error(d, f64::from(min), f64::from(max)));
            }
            true
        }
        Kind::Int { min, max, .. } => {
            let i = v.as_i64().ok_or_else(|| wrong("a whole number"))?;
            if !(min..=max).contains(&i) {
                #[allow(clippy::cast_precision_loss)] // registry bounds, small
                return Err(range_error(d, min as f64, max as f64));
            }
            true
        }
        Kind::Packet | Kind::Status => false,
    };
    if !fits {
        return Err(wrong("another kind of value"));
    }
    Ok(v)
}

fn range_error(d: &ParamDesc, min: f64, max: f64) -> ApiError {
    ApiError::bad(format!("{} must be between {min} and {max}", d.label))
        .with("min", json!(min))
        .with("max", json!(max))
}

/// `offsets` (`{serial: dB}`), each a finite number; only on a dB path.
fn offsets(d: &ParamDesc, msg: &Value) -> Result<Map<String, Value>, ApiError> {
    let Some(o) = msg.get("offsets").filter(|v| !v.is_null()) else {
        return Ok(Map::new());
    };
    if !is_db(d) {
        return Err(ApiError::bad(format!(
            "offsets are taken only on a path in dB; {} is not",
            d.path
        )));
    }
    let map = o
        .as_object()
        .ok_or_else(|| ApiError::bad("offsets is {serial: dB}"))?;
    if map.len() > crate::groups::MAX_MEMBERS {
        return Err(ApiError::bad("more offsets than members"));
    }
    if map
        .values()
        .any(|v| !v.as_f64().is_some_and(f64::is_finite))
    {
        return Err(ApiError::bad("an offset is a number of dB"));
    }
    Ok(map.clone())
}

/// A preset given as a slot (0 to 9) or a name, as `preset.load` takes it.
fn preset_ref(v: &Value) -> Option<Value> {
    match v {
        Value::Number(_) => v
            .as_u64()
            .filter(|s| *s < SLOTS)
            .map(|s| json!({"slot": s})),
        Value::String(n) if !n.is_empty() && n.len() <= NAME_BYTES => Some(json!({"name": n})),
        _ => None,
    }
}

/// `value + offset` clamped into `[lo, hi]`, and whether the clamp moved it.
fn clamp_sum(value: f64, offset: f64, lo: f64, hi: f64) -> (f64, bool) {
    let want = value + offset;
    let got = want.clamp(lo.min(hi), hi.max(lo));
    // f32 on the wire: a difference below its resolution is no clamp.
    (got, (got - want).abs() > 1e-4)
}

/// A refused row.
fn row_err(serial: &str, e: ApiError) -> Value {
    let mut row = json!({"serial": serial, "ok": false, "code": e.code, "msg": e.msg});
    if let Some(m) = row.as_object_mut() {
        for (k, v) in e.extra {
            if !matches!(k.as_str(), "serial" | "ok" | "code" | "msg") {
                m.insert(k, v);
            }
        }
    }
    row
}

/// A row from a single-device answer: its fields less `serial`, `path` and `idx`.
fn row_ok(serial: &str, answer: Value, keep: &[&str]) -> Value {
    let mut row = Map::new();
    row.insert("serial".into(), json!(serial));
    row.insert("ok".into(), json!(true));
    if let Value::Object(a) = answer {
        for (k, v) in a {
            if keep.contains(&k.as_str()) {
                row.insert(k, v);
            }
        }
    }
    Value::Object(row)
}

// ── The run ─────────────────────────────────────────────────────────────────

/// What a run does to each member.
enum Action<'a> {
    Get {
        path: &'a str,
        idx: &'a Value,
    },
    Set {
        d: &'static ParamDesc,
        idx: &'a Value,
        value: &'a Value,
        offsets: &'a Map<String, Value>,
    },
    Load {
        base: &'a Value,
        map: &'a Map<String, Value>,
    },
}

impl GroupParams {
    fn handle(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        t: &str,
        msg: &Value,
    ) -> Result<Value, ApiError> {
        if t != "group.param.get" {
            require(session, Role::Control)?;
        }
        let address = address(msg)?;
        match t {
            "group.param.get" => {
                let (d, idx) = path_and_idx(msg)?;
                if !d.is_readable() {
                    return Err(ApiError::bad(format!("{} cannot be read", d.path)));
                }
                let (members, from_group) = self.members(&address)?;
                let results = self.run(
                    hub,
                    session,
                    &members,
                    from_group,
                    &Action::Get {
                        path: d.path,
                        idx: &idx,
                    },
                )?;
                Ok(json!({"path": d.path, "idx": idx, "results": results}))
            }
            "group.param.set" => {
                let (d, idx) = path_and_idx(msg)?;
                let value = check_value(d, msg.get("value"))?;
                let offsets = offsets(d, msg)?;
                let (members, from_group) = self.members(&address)?;
                only_members(&offsets, &members, "offsets")?;
                let results = self.run(
                    hub,
                    session,
                    &members,
                    from_group,
                    &Action::Set {
                        d,
                        idx: &idx,
                        value: &value,
                        offsets: &offsets,
                    },
                )?;
                Ok(json!({"path": d.path, "idx": idx, "results": results}))
            }
            // "group.preset.load"
            _ => {
                let given = |k: &str| msg.get(k).filter(|v| !v.is_null());
                let base = match (given("slot"), given("name")) {
                    (Some(_), Some(_)) => {
                        return Err(ApiError::bad("slot or name, not both"));
                    }
                    (Some(s), None) => preset_ref(s)
                        .filter(|_| s.is_number())
                        .ok_or_else(|| ApiError::bad("slot is a whole number 0 to 9"))?,
                    (None, Some(n)) => {
                        preset_ref(n).filter(|_| n.is_string()).ok_or_else(|| {
                            ApiError::bad(format!("name is text of 1 to {NAME_BYTES} bytes"))
                        })?
                    }
                    (None, None) => return Err(ApiError::bad("slot or name is required")),
                };
                let map = match given("map") {
                    None => Map::new(),
                    Some(Value::Object(m)) if m.len() <= crate::groups::MAX_MEMBERS => {
                        let mut out = Map::new();
                        for (serial, v) in m {
                            let r = preset_ref(v).ok_or_else(|| {
                                ApiError::bad(format!(
                                    "map gives {serial} a slot 0 to 9 or a preset's name"
                                ))
                            })?;
                            out.insert(serial.clone(), r);
                        }
                        out
                    }
                    Some(_) => return Err(ApiError::bad("map is {serial: slot or name}")),
                };
                let (members, from_group) = self.members(&address)?;
                only_members(&map, &members, "map")?;
                let results = self.run(
                    hub,
                    session,
                    &members,
                    from_group,
                    &Action::Load {
                        base: &base,
                        map: &map,
                    },
                )?;
                Ok(json!({ "results": results }))
            }
        }
    }

    /// The devices, in order, and whether they are a group's (inventory serials).
    fn members(&self, address: &Address) -> Result<(Vec<String>, bool), ApiError> {
        match address {
            Address::Targets(t) => Ok((t.clone(), false)),
            Address::Group(id) => self
                .groups
                .as_ref()
                .and_then(|g| g.members(id))
                .map(|m| (m, true))
                .ok_or_else(|| ApiError::new("not_found", format!("no group {id}"))),
        }
    }

    /// Each member in turn, one row each.
    fn run(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        members: &[String],
        from_group: bool,
        action: &Action<'_>,
    ) -> Result<Vec<Value>, ApiError> {
        if members.len() > MAX_RESULT_ROWS {
            return Err(ApiError::bad(format!(
                "a run answers at most {MAX_RESULT_ROWS} rows"
            )));
        }
        let _running = self.running.enter(session)?;
        Ok(members
            .iter()
            .map(
                |serial| match self.row(hub, session, serial, from_group, action) {
                    Ok(row) => row,
                    Err(e) => row_err(serial, e),
                },
            )
            .collect())
    }

    /// One member.
    fn row(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        serial: &str,
        from_group: bool,
        action: &Action<'_>,
    ) -> Result<Value, ApiError> {
        let handle = online_handle(hub, session, serial, from_group)?;
        match action {
            Action::Get { path, idx } => {
                let ask = json!({"serial": serial, "path": path, "idx": idx});
                let answer = super::params::handle(&self.exec, hub, session, "param.get", &ask)?;
                Ok(row_ok(serial, answer, &["value", "choice", "value_b64"]))
            }
            Action::Set {
                d,
                idx,
                value,
                offsets,
            } => {
                no_guard(hub, handle)?;
                let (value, clamped) = if is_db(d) {
                    let base = value.as_f64().unwrap_or_default();
                    let offset = offsets.get(serial).and_then(Value::as_f64).unwrap_or(0.0);
                    let (lo, hi) = self
                        .exec
                        .run(hub, session, serial, |cx| Ok(cx.reader()?.write_range(d)))?
                        .ok_or_else(|| ApiError::internal("no range for a number in dB"))?;
                    let (v, moved) = clamp_sum(base, offset, lo, hi);
                    (json!(v), moved)
                } else {
                    ((*value).clone(), false)
                };
                let ask = json!({"serial": serial, "path": d.path, "idx": idx, "value": value});
                let answer = super::params::handle(&self.exec, hub, session, "param.set", &ask)?;
                let mut row = row_ok(serial, answer, &["value", "choice", "value_b64"]);
                if clamped && let Some(m) = row.as_object_mut() {
                    m.insert("clamped".into(), json!(true));
                }
                Ok(row)
            }
            Action::Load { base, map } => {
                no_guard(hub, handle)?;
                let mut ask = map.get(serial).cloned().unwrap_or_else(|| (*base).clone());
                if let Some(m) = ask.as_object_mut() {
                    m.insert("serial".into(), json!(serial));
                }
                let answer = super::presets::handle(&self.exec, hub, session, "preset.load", &ask)?;
                Ok(row_ok(serial, answer, &["slot", "name"]))
            }
        }
    }
}

/// A serial of `offsets` or `map` the request does not address is `bad_request`.
fn only_members(map: &Map<String, Value>, members: &[String], what: &str) -> Result<(), ApiError> {
    match map.keys().find(|k| !members.contains(k)) {
        Some(stray) => Err(ApiError::bad(format!(
            "{what} names {stray}, which the request does not address"
        ))),
        None => Ok(()),
    }
}

/// The member's handle while it is online. `offline` for a device the hub knows but
/// has not attached (a group's members are inventory serials; a target the inventory
/// holds), `not_found` for a target it does not know at all. Nothing waits.
fn online_handle(
    hub: &HubHandle,
    session: &Arc<Session>,
    serial: &str,
    from_group: bool,
) -> Result<u8, ApiError> {
    let devices = hub.devices();
    if let Some(d) = devices.iter().find(|d| d.identity.serial == serial) {
        return match d.state {
            DeviceState::Online => Ok(d.handle),
            _ => Err(ApiError::offline()),
        };
    }
    if from_group || in_inventory(hub, session, serial) {
        return Err(ApiError::offline());
    }
    Err(ApiError::new(
        "not_found",
        format!("no device {serial} is known"),
    ))
}

/// Whether the inventory (if this hub runs one) holds `serial`.
fn in_inventory(hub: &HubHandle, session: &Arc<Session>, serial: &str) -> bool {
    let ask = json!({"t": "inventory.list", "id": 0, "serial": serial});
    hub.service_request(session, &ask)
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|v| v.get("t").and_then(Value::as_str) == Some("ok"))
}

/// `conflict` while any guarded apply is pending on the device, the caller's own
/// included (`hub-api.md` 20.1): a fan-out does not write under a guard.
fn no_guard(hub: &HubHandle, handle: u8) -> Result<(), ApiError> {
    let status = hub.apply_status(handle);
    if status.get("active").is_some_and(|a| !a.is_null()) {
        return Err(ApiError::new(
            "conflict",
            "a guarded apply is pending on this device; confirm or cancel it first",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests_wp_p3gp {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn wp_p3gp_addressing_is_group_or_targets_exactly_once() {
        assert_eq!(
            address(&json!({"group": "g1"})).unwrap(),
            Address::Group("g1".into())
        );
        assert_eq!(
            address(&json!({"targets": ["A", "B"]})).unwrap(),
            Address::Targets(vec!["A".into(), "B".into()])
        );
        for bad in [
            json!({}),
            json!({"group": "g1", "targets": ["A"]}),
            json!({"targets": []}),
            json!({"targets": ["A", "A"]}),
            json!({"targets": ["group:g1"]}),
            json!({"targets": "A"}),
            json!({"group": "g 1"}),
            json!({"group": 1}),
            json!({"targets": (0..17).map(|i| format!("S{i}")).collect::<Vec<_>>()}),
        ] {
            assert_eq!(address(&bad).unwrap_err().code, "bad_request", "{bad}");
        }
        assert!(
            address(&json!({"targets": (0..16).map(|i| format!("S{i}")).collect::<Vec<_>>()}))
                .is_ok()
        );
    }

    #[test]
    fn wp_p3gp_values_offsets_and_gated_paths_are_checked_for_the_request() {
        let d = by_path("vol.master").unwrap();
        assert!(check_value(d, Some(&json!(-20.0))).is_ok());
        let e = check_value(d, Some(&json!(5.0))).unwrap_err();
        assert_eq!(e.code, "bad_request");
        assert!(e.extra.contains_key("max"));
        assert_eq!(
            check_value(d, Some(&json!("loud"))).unwrap_err().code,
            "bad_request"
        );
        let mute = by_path("vol.mute").unwrap();
        assert!(check_value(mute, Some(&json!(true))).is_ok());
        assert!(check_value(mute, Some(&json!(1))).is_err());
        // Offsets only in dB.
        assert!(offsets(d, &json!({"offsets": {"A": -3.0}})).is_ok());
        let e = offsets(mute, &json!({"offsets": {"A": 1}})).unwrap_err();
        assert_eq!(e.code, "bad_request");
        assert!(offsets(d, &json!({"offsets": {"A": "x"}})).is_err());
        // A gated path or one the API never writes.
        for path in ["preset.save", "meters.clear", "dev.reset"] {
            if let Some(d) = by_path(path) {
                assert_eq!(
                    check_value(d, Some(&Value::Null)).unwrap_err().code,
                    "bad_request",
                    "{path}"
                );
            }
        }
    }

    #[test]
    fn wp_p3gp_the_sum_is_clamped_into_the_devices_own_range() {
        assert_eq!(clamp_sum(-20.0, -3.0, -128.0, 0.0), (-23.0, false));
        assert_eq!(clamp_sum(-2.0, 5.0, -128.0, 0.0), (0.0, true));
        // An RP2040's input trim stops at +18 dB, an RP2350's at +24.
        assert_eq!(clamp_sum(20.0, 0.0, -60.0, 18.0), (18.0, true));
        assert_eq!(clamp_sum(20.0, 0.0, -60.0, 24.0), (20.0, false));
        assert_eq!(clamp_sum(-50.0, -20.0, -60.0, 24.0), (-60.0, true));
    }

    #[test]
    fn wp_p3gp_presets_are_a_slot_or_a_name() {
        assert_eq!(preset_ref(&json!(3)), Some(json!({"slot": 3})));
        assert_eq!(preset_ref(&json!("Show")), Some(json!({"name": "Show"})));
        assert_eq!(preset_ref(&json!(10)), None);
        assert_eq!(preset_ref(&json!("")), None);
        assert_eq!(preset_ref(&json!("x".repeat(32))), None);
        assert_eq!(preset_ref(&json!(-1)), None);
    }

    #[test]
    fn wp_p3gp_a_row_carries_its_code_and_fields() {
        let row = row_err(
            "A",
            ApiError::new("limit_exceeded", "over").with("limit", json!("master_volume")),
        );
        assert_eq!(
            row,
            json!({"serial": "A", "ok": false, "code": "limit_exceeded", "msg": "over",
                "limit": "master_volume"})
        );
        let row = row_ok(
            "A",
            json!({"serial": "A", "path": "vol.master", "idx": [], "value": -3.0}),
            &["value"],
        );
        assert_eq!(row, json!({"serial": "A", "ok": true, "value": -3.0}));
    }
}
