//! `hub.stats` and `hub.rename` (spec 7.9; WP P2HS, `docs/integrator/hub-api.md`
//! section 15). Admin only; every other role is `denied`.
//!
//! - `hub.stats`: `sessions` (network sessions open now, principals included, not the
//!   app's own), `uptime_s` (since the hub started, on the services' clock) and one row
//!   per attached device, by handle: `{handle, serial, cmds, errors, avg_rtt_ms,
//!   notify_dropped}`. The counts are the attachment's, since it attached
//!   (`device.rs`, `DeviceCounters`): commands that ran on the device, those whose
//!   transfer failed, their mean time on the transport, and the device's notifications
//!   that full session outboxes dropped. `serial` is our addition (section 17).
//! - `hub.rename {name}`: control characters are dropped and the ends trimmed, as the
//!   embedders do with a start name; empty or past [`MAX_NAME_CHARS`] is
//!   `bad_request`. The name is in use at once (the next `hello`, `/dspi/v1/info`),
//!   kept in the store for the next start, and announced again over mDNS.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session};

/// The cap in `hello.caps`.
pub(crate) const CAP: &str = "hub_stats";
/// Answers carry it (hub-api.md section 1.1).
const VERSION: u32 = 1;
/// Longest hub name, in characters: the embedders' own limit (`dspi-capi`).
pub(crate) const MAX_NAME_CHARS: usize = 64;

/// The service, unless [`ServiceConfig::stats`] is off.
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    config.stats.then(|| {
        Arc::new(Stats {
            clock: Arc::clone(setup.clock),
            started: setup.clock.now(),
        }) as Arc<dyn HubService>
    })
}

struct Stats {
    clock: Arc<dyn HubClock>,
    started: Instant,
}

/// One device's row of `hub.stats`.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    handle: u8,
    serial: String,
    cmds: u64,
    errors: u64,
    avg_rtt_ms: f64,
    notify_dropped: u64,
}

impl Row {
    fn to_json(&self) -> Value {
        json!({
            "handle": self.handle,
            "serial": self.serial,
            "cmds": self.cmds,
            "errors": self.errors,
            // Microseconds: past that is noise.
            "avg_rtt_ms": (self.avg_rtt_ms * 1000.0).round() / 1000.0,
            "notify_dropped": self.notify_dropped,
        })
    }
}

/// The body of `hub.stats`: rows by handle.
fn stats_body(sessions: usize, uptime_s: u64, mut rows: Vec<Row>) -> Value {
    rows.sort_by_key(|r| r.handle);
    json!({
        "sessions": sessions,
        "uptime_s": uptime_s,
        "devices": rows.iter().map(Row::to_json).collect::<Vec<_>>(),
    })
}

/// Why a name is refused, for `bad_request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameRefused {
    Missing,
    Empty,
    TooLong,
}

impl NameRefused {
    fn msg(self) -> String {
        match self {
            Self::Missing => "name must be a string".to_owned(),
            Self::Empty => "name is empty".to_owned(),
            Self::TooLong => format!("name is at most {MAX_NAME_CHARS} characters"),
        }
    }
}

/// The name of a `hub.rename`, cleaned: control characters dropped, the ends trimmed.
fn clean_name(msg: &Value) -> Result<String, NameRefused> {
    let raw = msg
        .get("name")
        .and_then(Value::as_str)
        .ok_or(NameRefused::Missing)?;
    let kept: String = raw.chars().filter(|c| !c.is_control()).collect();
    let name = kept.trim();
    if name.is_empty() {
        return Err(NameRefused::Empty);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(NameRefused::TooLong);
    }
    Ok(name.to_owned())
}

impl Stats {
    fn stats(&self, hub: &HubHandle) -> Value {
        let uptime_s = self
            .clock
            .now()
            .saturating_duration_since(self.started)
            .as_secs();
        let rows = hub
            .attached_devices()
            .iter()
            .map(|d| Row {
                handle: d.handle,
                serial: d.serial(),
                cmds: d.counters.cmds(),
                errors: d.counters.errors(),
                avg_rtt_ms: d.counters.avg_rtt_ms(),
                notify_dropped: d
                    .counters
                    .notify_dropped(hub.inner.relay.notify_dropped(d.handle)),
            })
            .collect();
        stats_body(hub.session_count(), uptime_s, rows)
    }
}

impl HubService for Stats {
    fn name(&self) -> &'static str {
        "stats"
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
        if t != "hub.stats" && t != "hub.rename" {
            return None;
        }
        let id = msg.get("id");
        if session.role() != Role::Admin {
            return Some(err(id, "denied", "admin only"));
        }
        Some(match t {
            "hub.stats" => ok(id, self.stats(hub)),
            _ => match clean_name(msg) {
                Ok(name) => {
                    hub.set_name(&name);
                    ok(id, json!({"name": name}))
                }
                Err(why) => err(id, "bad_request", &why.msg()),
            },
        })
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn ok(id: Option<&Value>, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p2hs {
    use super::*;

    fn row(handle: u8) -> Row {
        Row {
            handle,
            serial: format!("S{handle}"),
            cmds: 3,
            errors: 1,
            avg_rtt_ms: 1.23456,
            notify_dropped: 2,
        }
    }

    #[test]
    fn wp_p2hs_the_body_has_the_spec_fields_and_rows_by_handle() {
        let body = stats_body(3, 8812, vec![row(2), row(0)]);
        assert_eq!(body["sessions"], 3);
        assert_eq!(body["uptime_s"], 8812);
        let devices = body["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0]["handle"], 0);
        assert_eq!(devices[1]["handle"], 2);
        assert_eq!(devices[0]["serial"], "S0");
        assert_eq!(devices[0]["cmds"], 3);
        assert_eq!(devices[0]["errors"], 1);
        assert_eq!(devices[0]["avg_rtt_ms"], 1.235);
        assert_eq!(devices[0]["notify_dropped"], 2);
    }

    #[test]
    fn wp_p2hs_a_name_is_cleaned_and_bounded() {
        let name = |v: Value| clean_name(&json!({"name": v}));
        assert_eq!(name(json!("  Studio Mac ")).unwrap(), "Studio Mac");
        assert_eq!(name(json!("a\nb\u{1b}c")).unwrap(), "abc");
        assert_eq!(name(json!("Scène ✓")).unwrap(), "Scène ✓");
        assert_eq!(name(json!(" \t\n")), Err(NameRefused::Empty));
        assert_eq!(name(json!(7)), Err(NameRefused::Missing));
        assert_eq!(clean_name(&json!({})), Err(NameRefused::Missing));
        let fits = "é".repeat(MAX_NAME_CHARS);
        assert_eq!(name(json!(fits.clone())).unwrap(), fits);
        let long = "é".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(name(json!(long)), Err(NameRefused::TooLong));
    }

    #[test]
    fn wp_p2hs_answers_carry_the_id_and_v() {
        let id = json!(14);
        let answer: Value = serde_json::from_str(&ok(Some(&id), json!({"name": "x"}))).unwrap();
        assert_eq!(answer, json!({"t": "ok", "id": 14, "v": 1, "name": "x"}));
        let refused: Value = serde_json::from_str(&err(None, "denied", "admin only")).unwrap();
        assert_eq!(refused["code"], "denied");
        assert_eq!(refused["id"], Value::Null);
    }
}
