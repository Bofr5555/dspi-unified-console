//! The hub's admin families (WP P2CC2, the second section of `hub_server`):
//! alarms, schedules, notifications out, MQTT and API keys, as typed calls.
//! The wire contract is `docs/integrator/hub-api.md` sections 7, 8, 12, 13.1
//! and 14.1; each call's name follows its message (`alarm.ack` is
//! [`DspiCore::hub_alarm_ack`]).
//!
//! - Alarms: [`DspiCore::hub_alarms`] (`alarm.list`, every role),
//!   [`DspiCore::hub_alarm_ack`] (Control, Admin),
//!   [`DspiCore::hub_alarm_rules_get`] / [`DspiCore::hub_alarm_rules_set`]
//!   (Admin to set), and `alarm.changed` through [`super::HubServerListener`]
//!   (kind `alarm.changed`, [`super::HubServerEvent::alarm`]). Each alarm code
//!   has a notice row `alarm.<code>` ([`ALARM_CODES`], [`hub_alarm_notice`]).
//! - Schedules: [`DspiCore::hub_schedules`] (`schedule.list`, every role),
//!   [`DspiCore::hub_schedule_set`], [`DspiCore::hub_schedule_delete`],
//!   [`DspiCore::hub_schedule_run_now`] (Admin), and `schedule.ran`
//!   ([`super::HubServerEvent::run`]).
//! - Notifications out (Admin): [`DspiCore::hub_outbound_get`],
//!   [`DspiCore::hub_outbound_set`], [`DspiCore::hub_outbound_test`].
//! - MQTT (Admin): [`DspiCore::hub_mqtt_get`], [`DspiCore::hub_mqtt_set`],
//!   [`DspiCore::hub_mqtt_status`].
//! - API keys (Admin): [`DspiCore::hub_api_keys`] (`auth.list`'s keys),
//!   [`DspiCore::hub_api_key_mint`] (`auth.mint`),
//!   [`DspiCore::hub_api_key_revoke`] (`auth.revoke`).
//!
//! As the first section: [`DspiError::Unsupported`] with nothing sent
//! without the family's cap, the hub's refusals typed, and the client
//! read-only mode lets out only the reads ([`super::HUB_SERVER_READS`]).
//! **Secrets** (a minted key, a webhook's new secret) come once, in the
//! answer of the call that made them: they are never logged. Each is a
//! [`HubSecret`] object (read with `reveal`), so neither `Debug` nor a
//! binding's generated record printing shows it; a password typed in
//! (`HubMqttSettings`) shows `set` in `Debug`.

use std::sync::Arc;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use super::{cut, small, text, texts};
use crate::{DspiCore, DspiError, Notice, Result};

/// Cap: `alarm.*` (hub-api.md section 7).
pub const CAP_ALARMS: &str = "alarms";
/// Cap: `outbound.*` (section 8).
pub const CAP_OUTBOUND: &str = "outbound";
/// Cap: `schedule.*` (section 12).
pub const CAP_SCHEDULE: &str = "schedule";
/// Cap: `mqtt.*` (section 13.1).
pub const CAP_MQTT: &str = "mqtt";
/// Cap: `auth.mint` and `kind` in `auth.list` (section 14.1).
pub const CAP_API_KEYS: &str = "api_keys";
/// Cap: the REST mirror, `/api/v1/` (section 14.2; only while the hub runs
/// with `--rest`). No call here uses it; [`super::HubCaps::rest`] tells it.
pub const CAP_REST: &str = "rest";

/// Every alarm code the hub raises: the device's (section 7) and the audio
/// link's (`audio-link.md`). Each has a notice row `alarm.<code>`.
pub const ALARM_CODES: [&str; 18] = [
    "device_offline",
    "unresponsive",
    "device_restarted",
    "clip",
    "temperature",
    "cpu",
    "schedule_failed",
    "clock_unset",
    "alink_lost",
    "unit_offline",
    "alink_errors",
    "reanchor",
    "system_muted",
    "delay_below_min",
    "usb_lock_error",
    "usb_realign_failed",
    "usb_setting_restored",
    "usb_realigned",
];

/// The notice rows' family: `alarm.<code>`.
pub const NOTICE_ALARM_FAMILY: &str = "alarm";

/// The notice row for a code this build does not know.
pub const NOTICE_ALARM_UNKNOWN: &str = "alarm.unknown";

/// `alarm.list`'s `state` filters.
const ALARM_STATES: [&str; 3] = ["active", "cleared", "all"];

/// Rows taken from the hub for one list (its own caps: 500 alarms, 256 rules,
/// 16 targets, 64 clients).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const ROWS_MAX: usize = 512;
/// Results of one schedule run (16 actions on 16 targets).
const RESULTS_MAX: usize = 256;
/// A PEM or a long text taken from the hub, in characters.
const PEM_MAX: usize = 32 * 1024;
/// A URL, in characters (the hub's own cap, 2048 bytes).
const URL_MAX: usize = 2048;

fn shown(set: bool) -> String {
    if set { "set" } else { "not set" }.to_owned()
}

fn long(v: Option<&Value>, max: usize) -> String {
    v.and_then(Value::as_str)
        .map(|t| cut(t, max))
        .unwrap_or_default()
}

fn flag(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

/// "" as JSON `null`, anything else as itself.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn or_null(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        Value::String(s.to_owned())
    }
}

// ── Alarms ─────────────────────────────────────────────────────────────────

/// One alarm of the hub's (`alarm.list`, `alarm.ack`, `alarm.changed`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubAlarm {
    /// `device:<n>` or `alink:<n>`, unique for the hub's run.
    pub id: String,
    /// `device` or `alink`.
    pub source: String,
    /// One of [`ALARM_CODES`] (or a code a newer hub adds).
    pub code: String,
    /// What it is about: the serial, a schedule rule's id, `hub`, or an
    /// audio link unit.
    pub subject: String,
    /// The device's serial; "" for an alarm about no one device.
    pub serial: String,
    /// `warning` or `critical`.
    pub severity: String,
    /// The hub's line for a person (also the notice's detail).
    pub message: String,
    /// `raised`, `acknowledged` or `cleared`.
    pub state: String,
    /// When it was raised (ISO-8601 UTC).
    pub raised_at: String,
    /// When it was cleared; "" while on.
    pub cleared_at: String,
    /// Who acknowledged it (a client's or principal's name); "" when none.
    pub acknowledged_by: String,
    /// How often it was raised again while on.
    pub count: u32,
    /// `device_restarted`'s reset reason (`watchdog`, `glitch`, `brownout`,
    /// ...), read from the hub's message; "" otherwise.
    pub reason: String,
    /// Its notice row: `alarm.<code>`, or [`NOTICE_ALARM_UNKNOWN`].
    pub notice_key: String,
}

/// The reset reason in `device_restarted`'s message, "Restarted at <time>
/// (<reason>: <words>)"; "" when it is not there.
fn restart_reason(message: &str) -> String {
    message
        .rfind(" (")
        .and_then(|i| message.get(i.saturating_add(2)..))
        .and_then(|rest| rest.split([':', ')']).next())
        .filter(|r| {
            !r.is_empty() && r.len() <= 32 && r.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
        .map(str::to_owned)
        .unwrap_or_default()
}

fn alarm_notice_key(code: &str) -> String {
    if ALARM_CODES.contains(&code) {
        format!("{NOTICE_ALARM_FAMILY}.{code}")
    } else {
        NOTICE_ALARM_UNKNOWN.to_owned()
    }
}

pub(crate) fn alarm_from(v: &Value) -> HubAlarm {
    let code = text(v.get("code"));
    let message = long(v.get("message"), 1024);
    let reason = if code == "device_restarted" {
        restart_reason(&message)
    } else {
        String::new()
    };
    let id = match v.get("id") {
        Some(Value::Number(n)) => n.to_string(),
        other => text(other),
    };
    HubAlarm {
        id,
        source: text(v.get("source")),
        notice_key: alarm_notice_key(&code),
        code,
        subject: text(v.get("subject")),
        serial: text(v.get("serial")),
        severity: text(v.get("severity")),
        message,
        state: text(v.get("state")),
        raised_at: text(v.get("raised_at")),
        cleared_at: text(v.get("cleared_at")),
        acknowledged_by: text(v.get("acknowledged_by")),
        count: small(v.get("count")).unwrap_or(0),
        reason,
    }
}

/// One code's alarm rule (`alarm.rules.get`/`set`). A field the code does not
/// have is `None`; in [`DspiCore::hub_alarm_rules_set`] a `None` keeps the
/// hub's value.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubAlarmRule {
    /// The alarm code.
    pub code: String,
    /// The rule is on (off clears its alarms).
    pub on: bool,
    /// `device_offline`, `unresponsive`: seconds (1 to 86 400).
    pub after_s: Option<u32>,
    /// `clip`: clip events within a minute (1 to 1000).
    pub per_minute: Option<u32>,
    /// `clip`: mute the device once per storm (off by default).
    pub mute_on_storm: Option<bool>,
    /// `temperature`: °C (0 to 150).
    pub above_c: Option<f64>,
    /// `cpu`: % (1 to 100).
    pub above_pct: Option<u32>,
    /// `cpu`: for so many seconds (1 to 3600).
    pub for_s: Option<u32>,
}

/// The rules object as rows, in [`ALARM_CODES`]' order, then any code a newer
/// hub adds.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn rules_from(v: &Value) -> Vec<HubAlarmRule> {
    let Some(map) = v.as_object() else {
        return Vec::new();
    };
    let row = |code: &str, r: &Value| HubAlarmRule {
        code: cut(code, 64),
        on: flag(r.get("on")),
        after_s: small(r.get("after_s")),
        per_minute: small(r.get("per_minute")),
        mute_on_storm: r.get("mute_on_storm").and_then(Value::as_bool),
        above_c: r.get("above_c").and_then(Value::as_f64),
        above_pct: small(r.get("above_pct")),
        for_s: small(r.get("for_s")),
    };
    let mut out: Vec<HubAlarmRule> = ALARM_CODES
        .iter()
        .filter_map(|c| map.get(*c).map(|r| row(c, r)))
        .collect();
    out.extend(
        map.iter()
            .filter(|(c, _)| !ALARM_CODES.contains(&c.as_str()))
            .take(64)
            .map(|(c, r)| row(c, r)),
    );
    out
}

/// The rows as `alarm.rules.set`'s `rules`: only the fields given.
pub(crate) fn rules_to(rules: &[HubAlarmRule]) -> Result<Value> {
    let mut all = Map::new();
    for r in rules {
        if r.code.is_empty() {
            return Err(DspiError::InvalidArgument {
                msg: "an alarm rule needs its code".into(),
            });
        }
        let mut m = Map::new();
        m.insert("on".into(), r.on.into());
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                m.insert(k.into(), v);
            }
        };
        put("after_s", r.after_s.map(Into::into));
        put("per_minute", r.per_minute.map(Into::into));
        put("mute_on_storm", r.mute_on_storm.map(Into::into));
        put(
            "above_c",
            r.above_c
                .and_then(|c| serde_json::Number::from_f64(c).map(Value::Number)),
        );
        put("above_pct", r.above_pct.map(Into::into));
        put("for_s", r.for_s.map(Into::into));
        if all.insert(r.code.clone(), Value::Object(m)).is_some() {
            return Err(DspiError::InvalidArgument {
                msg: format!("alarm rule {} given twice", r.code),
            });
        }
    }
    Ok(Value::Object(all))
}

/// The reset reason's words in a notice ([`hub_alarm_notice`]).
fn reason_words(reason: &str) -> String {
    match reason {
        "power_on" => "power-on",
        "brownout" => "brown-out",
        "run_pin" => "run pin",
        "watchdog" => "watchdog",
        "software" => "software reset",
        "debugger" => "debugger",
        "glitch" => "glitch",
        "" | "unknown" => "unknown cause",
        other => other,
    }
    .to_owned()
}

/// What to tell the user about `alarm`: its row `alarm.<code>`, with
/// `{time}` the client's own rendering of [`HubAlarm::raised_at`] (`time`;
/// "" leaves the hub's ISO time) and `{reason}` the reset's cause in words.
/// The detail is the hub's message. A code this build does not know gets
/// [`NOTICE_ALARM_UNKNOWN`]'s row.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn hub_alarm_notice(alarm: HubAlarm, time: String) -> Notice {
    let time = if time.is_empty() {
        alarm.raised_at.clone()
    } else {
        time
    };
    let reason = reason_words(&alarm.reason);
    let fill = |s: String| s.replace("{time}", &time).replace("{reason}", &reason);
    let n = crate::notice_text::notice_for_key(alarm_notice_key(&alarm.code), alarm.message);
    Notice {
        title: fill(n.title),
        reason: fill(n.reason),
        action: fill(n.action),
        ..n
    }
}

// ── Schedules ──────────────────────────────────────────────────────────────

/// One action of a schedule rule. Which fields count depends on `t`:
/// `preset.load` (`slot` or `name`), `snapshot.recall` (`snapshot` or
/// `name`), `system.scene.apply` (`scene`), `param.set` (`path`, `idx`, and
/// `value` for a number or `flag` for `vol.mute`/`out.mute`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubScheduleAction {
    /// The message: `preset.load`, `snapshot.recall`, `system.scene.apply`
    /// or `param.set`.
    pub t: String,
    /// `preset.load`'s slot (0 to 9).
    pub slot: Option<u8>,
    /// A preset's or a snapshot's name ("" for none).
    pub name: String,
    /// `snapshot.recall`'s snapshot id.
    pub snapshot: String,
    /// `system.scene.apply`'s scene.
    pub scene: String,
    /// `param.set`'s path: `vol.master`, `vol.user`, `vol.mute`, `out.mute`.
    pub path: String,
    /// `param.set`'s indices.
    pub idx: Vec<u32>,
    /// `param.set`'s value, a number (dB).
    pub value: Option<f64>,
    /// `param.set`'s value, true or false (a mute).
    pub flag: Option<bool>,
}

/// What a rule last did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubScheduleLast {
    /// When (ISO-8601 UTC).
    pub at: String,
    /// Every action went through.
    pub ok: bool,
    /// The time was skipped (a clock jump).
    pub skipped: bool,
}

/// A schedule rule (`schedule.list`, `schedule.set`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubScheduleRule {
    /// The hub's id (`r1`, ...); "" for a new rule.
    pub id: String,
    /// 1 to 48 characters.
    pub name: String,
    /// On its schedule (`run_now` runs it either way).
    pub enabled: bool,
    /// Weekly: `mon` to `sun` (with [`HubScheduleRule::time`]); empty for a
    /// one-off.
    pub days: Vec<String>,
    /// Weekly: local `HH:MM`.
    pub time: String,
    /// One-off: a local `YYYY-MM-DDTHH:MM` ("" for a weekly rule).
    pub once: String,
    /// 1 to 16.
    pub actions: Vec<HubScheduleAction>,
    /// At most 16 serials.
    pub targets: Vec<String>,
    /// The next run (ISO-8601 UTC); "" when none within a week or while the
    /// clock is unset. Read only.
    pub next_run: String,
    /// The last run. Read only.
    pub last: Option<HubScheduleLast>,
}

/// The hub's schedule (`schedule.list`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubSchedules {
    /// The rules.
    pub rules: Vec<HubScheduleRule>,
    /// The time zone the rules are read in.
    pub zone: String,
    /// The hub's local time now (`YYYY-MM-DDTHH:MM`).
    pub local_time: String,
    /// The hub's clock reads before its build date: nothing runs.
    pub clock_unset: bool,
}

/// One row of a run: an action on a target.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubScheduleResult {
    /// The device ("" for a scene refused as a whole).
    pub serial: String,
    /// The action, by position and `t` (`0:preset.load`).
    pub action: String,
    /// It went through.
    pub ok: bool,
    /// The hub's code when not (`offline`, `limit_exceeded`, ...).
    pub code: String,
    /// Why, for a person.
    pub msg: String,
}

/// A schedule's run (`schedule.run_now`'s answer, `schedule.ran`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubScheduleRun {
    /// The rule's id.
    pub rule: String,
    /// Its name.
    pub name: String,
    /// When (ISO-8601 UTC).
    pub at: String,
    /// The admin who ran it now; "" on its schedule.
    pub by: String,
    /// Each action on each target, in the rule's order.
    pub results: Vec<HubScheduleResult>,
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn action_from(v: &Value) -> HubScheduleAction {
    let (value, flag) = match v.get("value") {
        Some(Value::Bool(b)) => (None, Some(*b)),
        Some(n) => (n.as_f64(), None),
        None => (None, None),
    };
    HubScheduleAction {
        t: text(v.get("t")),
        slot: small(v.get("slot")),
        name: text(v.get("name")),
        snapshot: text(v.get("snapshot")),
        scene: text(v.get("scene")),
        path: text(v.get("path")),
        idx: v
            .get("idx")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(4).filter_map(|i| small(Some(i))).collect())
            .unwrap_or_default(),
        value,
        flag,
    }
}

fn action_to(a: &HubScheduleAction) -> Result<Value> {
    let mut m = Map::new();
    m.insert("t".into(), a.t.clone().into());
    match a.t.as_str() {
        "preset.load" => match a.slot {
            Some(s) => {
                m.insert("slot".into(), s.into());
            }
            None => {
                m.insert("name".into(), a.name.clone().into());
            }
        },
        "snapshot.recall" => {
            if a.snapshot.is_empty() {
                m.insert("name".into(), a.name.clone().into());
            } else {
                m.insert("snapshot".into(), a.snapshot.clone().into());
            }
        }
        "system.scene.apply" => {
            m.insert("scene".into(), a.scene.clone().into());
        }
        "param.set" => {
            m.insert("path".into(), a.path.clone().into());
            if !a.idx.is_empty() {
                m.insert("idx".into(), a.idx.clone().into());
            }
            let value = match (a.flag, a.value) {
                (Some(b), _) => Value::Bool(b),
                (None, Some(v)) => serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .ok_or_else(|| DspiError::InvalidArgument {
                        msg: "param.set: the value is not a number".into(),
                    })?,
                (None, None) => {
                    return Err(DspiError::InvalidArgument {
                        msg: "param.set needs a value".into(),
                    });
                }
            };
            m.insert("value".into(), value);
        }
        // The hub names the actions it runs.
        _ => {}
    }
    Ok(Value::Object(m))
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn rule_from(v: &Value) -> HubScheduleRule {
    let when = v.get("when").unwrap_or(&Value::Null);
    HubScheduleRule {
        id: text(v.get("id")),
        name: text(v.get("name")),
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        days: texts(when.get("days")),
        time: text(when.get("time")),
        once: text(when.get("once")),
        actions: v
            .get("actions")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(64).map(action_from).collect())
            .unwrap_or_default(),
        targets: texts(v.get("targets")),
        next_run: text(v.get("next_run")),
        last: v
            .get("last")
            .filter(|l| l.is_object())
            .map(|l| HubScheduleLast {
                at: text(l.get("at")),
                ok: flag(l.get("ok")),
                skipped: flag(l.get("skipped")),
            }),
    }
}

/// `schedule.set`'s `rule`: the rule without the hub's read-only fields.
pub(crate) fn rule_to(r: &HubScheduleRule) -> Result<Value> {
    let when = if r.once.is_empty() {
        json!({"days": r.days, "time": r.time})
    } else {
        if !r.days.is_empty() {
            return Err(DspiError::InvalidArgument {
                msg: "a rule runs on days at a time, or once: not both".into(),
            });
        }
        json!({"once": r.once})
    };
    let actions = r
        .actions
        .iter()
        .map(action_to)
        .collect::<Result<Vec<_>>>()?;
    let mut m = Map::new();
    if !r.id.is_empty() {
        m.insert("id".into(), r.id.clone().into());
    }
    m.insert("name".into(), r.name.clone().into());
    m.insert("enabled".into(), r.enabled.into());
    m.insert("when".into(), when);
    m.insert("actions".into(), actions.into());
    m.insert("targets".into(), r.targets.clone().into());
    Ok(Value::Object(m))
}

pub(crate) fn run_from(v: &Value) -> HubScheduleRun {
    let row = |r: &Value| HubScheduleResult {
        serial: text(r.get("serial")),
        action: text(r.get("action")),
        ok: flag(r.get("ok")),
        code: text(r.get("code")),
        msg: text(r.get("msg")),
    };
    HubScheduleRun {
        rule: text(v.get("rule")),
        name: text(v.get("name")),
        at: text(v.get("at")),
        by: text(v.get("by")),
        results: v
            .get("results")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(RESULTS_MAX).map(row).collect())
            .unwrap_or_default(),
    }
}

// ── Notifications out ──────────────────────────────────────────────────────

/// A target's last delivery try.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubOutboundLast {
    /// When (ISO-8601 UTC).
    pub at: String,
    /// It was delivered.
    pub ok: bool,
    /// The receiver's HTTP status, if it answered.
    pub status: Option<u16>,
    /// Why not, for a person.
    pub msg: String,
}

/// A notification target (`outbound.get`, `outbound.set`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubOutboundTarget {
    /// 1 to 32 of `A-Z a-z 0-9 _ -`; "" for a new one (the hub names it).
    pub id: String,
    /// At most 64 characters; "" for the id.
    pub name: String,
    /// `webhook` (a JSON POST) or `push` (an ntfy-style text POST).
    pub kind: String,
    /// `https://`, or `http://` with [`HubOutboundTarget::plaintext`].
    pub url: String,
    /// Deliveries go out.
    pub enabled: bool,
    /// Of `alarm.raised`, `alarm.cleared`, `restart`, `schedule.ran`,
    /// `schedule.failed`; empty sends none.
    pub kinds: Vec<String>,
    /// Read: `set` or `not set`. In [`DspiCore::hub_outbound_set`]: `new`
    /// (the hub makes one, shown once), `set` or "" (keep), `not set`
    /// (none). A secret is never typed.
    pub secret: String,
    /// `http://` to a private address, unencrypted.
    pub plaintext: bool,
    /// The CA or self-signed certificate(s) to trust instead of the public
    /// CAs (PEM); "" for none.
    pub ca_pem: String,
    /// The receiver's key pin (`sha256//<base64>`); "" for none.
    pub pin_sha256: String,
    /// Deliveries waiting. Read only.
    pub queued: u32,
    /// Deliveries dropped. Read only.
    pub dropped: u64,
    /// The last try. Read only.
    pub last: Option<HubOutboundLast>,
    /// Sent over TLS. Read only.
    pub encrypted: bool,
    /// `webpki`, `ca` or `pin` (https only); "" otherwise. Read only.
    pub trust: String,
    /// A notice for the Admin (a target kept from before TLS). Read only.
    pub notice: String,
}

// -- WP P2DFIXB -- (review p2d F3, PLAN-SEC S9) a secret is an object, so no
// generated record's toString/description prints it
/// A secret the hub made and shows once: an API key, a webhook's secret. An
/// object, not a string field, so that no binding's generated record
/// printing (Kotlin `toString`, Swift `description`) shows it: read it with
/// [`HubSecret::reveal`] to show it once, and keep it nowhere else. Its
/// `Debug` is `set`; serialized (the C and wasm answers) it is the plain
/// string.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
#[derive(Default, PartialEq, Eq)]
pub struct HubSecret(String);

impl HubSecret {
    /// `text` as a secret.
    pub fn new(text: String) -> Arc<Self> {
        Arc::new(Self(text))
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl HubSecret {
    /// The secret's text, to show once. No I/O.
    pub fn reveal(&self) -> String {
        self.0.clone()
    }
}

impl std::fmt::Debug for HubSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&shown(!self.0.is_empty()))
    }
}

impl Serialize for HubSecret {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// A secret field as its plain string (serde has no `Arc` without `rc`).
fn reveal_ser<S: Serializer>(v: &Arc<HubSecret>, s: S) -> std::result::Result<S::Ok, S::Error> {
    v.as_ref().serialize(s)
}

/// A secret the hub made for a target, shown once ([`HubOutboundSet`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubOutboundSecret {
    /// The target's id.
    pub target: String,
    /// The secret (64 hex characters): show it once, keep it nowhere else.
    #[serde(serialize_with = "reveal_ser")]
    pub secret: Arc<HubSecret>,
}

/// `outbound.set`'s answer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubOutboundSet {
    /// The targets now.
    pub targets: Vec<HubOutboundTarget>,
    /// The secrets made by this call: in this answer only.
    pub secrets: Vec<HubOutboundSecret>,
}

/// `outbound.test`'s answer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubOutboundTest {
    /// It was delivered.
    pub delivered: bool,
    /// The receiver's HTTP status, if it answered.
    pub status: Option<u16>,
    /// Why not, for a person.
    pub msg: String,
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn target_from(v: &Value) -> HubOutboundTarget {
    HubOutboundTarget {
        id: text(v.get("id")),
        name: text(v.get("name")),
        kind: text(v.get("kind")),
        url: long(v.get("url"), URL_MAX),
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        kinds: texts(v.get("kinds")),
        secret: text(v.get("secret")),
        plaintext: flag(v.get("plaintext")),
        ca_pem: long(v.get("ca_pem"), PEM_MAX),
        pin_sha256: text(v.get("pin_sha256")),
        queued: small(v.get("queued")).unwrap_or(0),
        dropped: v.get("dropped").and_then(Value::as_u64).unwrap_or(0),
        last: v
            .get("last")
            .filter(|l| l.is_object())
            .map(|l| HubOutboundLast {
                at: text(l.get("at")),
                ok: flag(l.get("ok")),
                status: small(l.get("status")),
                msg: text(l.get("msg")),
            }),
        encrypted: flag(v.get("encrypted")),
        trust: text(v.get("trust")),
        notice: text(v.get("notice")),
    }
}

/// `outbound.set`'s entry: the settable fields only.
pub(crate) fn target_to(t: &HubOutboundTarget) -> Value {
    let mut m = Map::new();
    if !t.id.is_empty() {
        m.insert("id".into(), t.id.clone().into());
    }
    if !t.name.is_empty() {
        m.insert("name".into(), t.name.clone().into());
    }
    m.insert("kind".into(), t.kind.clone().into());
    m.insert("url".into(), t.url.clone().into());
    m.insert("enabled".into(), t.enabled.into());
    m.insert("kinds".into(), t.kinds.clone().into());
    if !t.secret.is_empty() {
        m.insert("secret".into(), t.secret.clone().into());
    }
    m.insert("plaintext".into(), t.plaintext.into());
    if !t.ca_pem.is_empty() {
        m.insert("ca_pem".into(), t.ca_pem.clone().into());
    }
    if !t.pin_sha256.is_empty() {
        m.insert("pin_sha256".into(), t.pin_sha256.clone().into());
    }
    Value::Object(m)
}

// ── MQTT ───────────────────────────────────────────────────────────────────

/// TLS to the broker (`mqtt.get`'s `tls`). PEMs and file paths "" for none.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubMqttTls {
    /// `on` or `off` ("" is off).
    pub mode: String,
    /// The CA or self-signed certificate(s) to trust (PEM).
    pub ca_pem: String,
    /// The broker's key pin.
    pub pin_sha256: String,
    /// A client certificate (PEM).
    pub client_cert_pem: String,
    /// Its key. Read: `set` or `not set`; send `set` back to keep it, a PEM
    /// to set it, "" or `not set` to clear it.
    pub client_key_pem: String,
    /// A client certificate's file on the hub's machine.
    pub client_cert_file: String,
    /// Its key's file (0600) on the hub's machine.
    pub client_key_file: String,
    /// Without TLS, send the password to a broker not on loopback too.
    pub allow_plaintext_password: bool,
}

impl std::fmt::Debug for HubMqttTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = match self.client_key_pem.as_str() {
            "" | "not set" => "not set",
            _ => "set",
        };
        f.debug_struct("HubMqttTls")
            .field("mode", &self.mode)
            .field("ca_pem", &shown(!self.ca_pem.is_empty()))
            .field("pin_sha256", &self.pin_sha256)
            .field("client_cert_pem", &shown(!self.client_cert_pem.is_empty()))
            .field("client_key_pem", &key)
            .field("client_cert_file", &self.client_cert_file)
            .field("client_key_file", &self.client_key_file)
            .field("allow_plaintext_password", &self.allow_plaintext_password)
            .finish()
    }
}

/// The broker setting (`mqtt.get`, `mqtt.set`). In
/// [`DspiCore::hub_mqtt_set`] "" in `client_id`, `base_topic` (and a `None`
/// `port`) is the hub's default, "" in `broker`/`username` none.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubMqttSettings {
    /// The hub connects and publishes.
    pub enabled: bool,
    /// The broker's host name or address.
    pub broker: String,
    /// Its port (read: the one in use).
    pub port: Option<u16>,
    /// The client id (read: the one in use).
    pub client_id: String,
    /// The user name; "" for none.
    pub username: String,
    /// Read: `set` or `not set`. To set: the password itself (typed by the
    /// Admin); `set` or "" keeps it; `not set` clears it. Never logged.
    pub password: String,
    /// The base topic (read: the one in use).
    pub base_topic: String,
    /// TLS.
    pub tls: HubMqttTls,
    /// Home Assistant discovery and `set` topics on (section 13.2).
    pub ha_enabled: bool,
    /// The API key (its `cid`) Home Assistant's `set` topics act as.
    pub ha_key: Option<u32>,
    /// A one-time notice of this run (a file migrated). Read only.
    pub notice: String,
}

impl std::fmt::Debug for HubMqttSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let password = match self.password.as_str() {
            "" | "not set" => "not set",
            _ => "set",
        };
        f.debug_struct("HubMqttSettings")
            .field("enabled", &self.enabled)
            .field("broker", &self.broker)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field("password", &password)
            .field("base_topic", &self.base_topic)
            .field("tls", &self.tls)
            .field("ha_enabled", &self.ha_enabled)
            .field("ha_key", &self.ha_key)
            .field("notice", &self.notice)
            .finish()
    }
}

/// The broker connection now (`mqtt.status`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubMqttStatus {
    /// `off`, `connecting`, `connected` or `failed`.
    pub state: String,
    /// The last reason, for a person (a refused password).
    pub msg: String,
    /// When `state` last changed (ISO-8601 UTC).
    pub since: String,
    /// Publishes waiting while the broker is away.
    pub queued: u32,
    /// Publishes dropped past that.
    pub dropped: u64,
    /// The connection is TLS.
    pub encrypted: bool,
    /// A one-time notice of this run.
    pub notice: String,
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn mqtt_from(v: &Value, notice: Option<&Value>) -> HubMqttSettings {
    let tls = v.get("tls").unwrap_or(&Value::Null);
    let ha = v.get("ha").unwrap_or(&Value::Null);
    HubMqttSettings {
        enabled: flag(v.get("enabled")),
        broker: text(v.get("broker")),
        port: small(v.get("port")),
        client_id: text(v.get("client_id")),
        username: text(v.get("username")),
        password: text(v.get("password")),
        base_topic: text(v.get("base_topic")),
        tls: HubMqttTls {
            mode: text(tls.get("mode")),
            ca_pem: long(tls.get("ca_pem"), PEM_MAX),
            pin_sha256: text(tls.get("pin_sha256")),
            client_cert_pem: long(tls.get("client_cert_pem"), PEM_MAX),
            client_key_pem: text(tls.get("client_key_pem")),
            client_cert_file: long(tls.get("client_cert_file"), 4096),
            client_key_file: long(tls.get("client_key_file"), 4096),
            allow_plaintext_password: flag(tls.get("allow_plaintext_password")),
        },
        ha_enabled: flag(ha.get("enabled")),
        ha_key: small(ha.get("key")),
        notice: text(notice),
    }
}

/// `mqtt.set`'s `settings`: every field (the hub reads `tls` whole).
pub(crate) fn mqtt_to(s: &HubMqttSettings) -> Value {
    let t = &s.tls;
    let mut tls = Map::new();
    tls.insert(
        "mode".into(),
        if t.mode.is_empty() { "off" } else { &t.mode }.into(),
    );
    for (k, v) in [
        ("ca_pem", &t.ca_pem),
        ("pin_sha256", &t.pin_sha256),
        ("client_cert_pem", &t.client_cert_pem),
        ("client_key_pem", &t.client_key_pem),
        ("client_cert_file", &t.client_cert_file),
        ("client_key_file", &t.client_key_file),
    ] {
        tls.insert(k.into(), or_null(v));
    }
    tls.insert(
        "allow_plaintext_password".into(),
        t.allow_plaintext_password.into(),
    );
    let mut m = Map::new();
    m.insert("enabled".into(), s.enabled.into());
    m.insert("broker".into(), or_null(&s.broker));
    m.insert("port".into(), s.port.map_or(Value::Null, Into::into));
    m.insert("client_id".into(), or_null(&s.client_id));
    m.insert("username".into(), or_null(&s.username));
    if !s.password.is_empty() {
        m.insert("password".into(), s.password.clone().into());
    }
    m.insert("base_topic".into(), or_null(&s.base_topic));
    m.insert("tls".into(), Value::Object(tls));
    m.insert(
        "ha".into(),
        json!({"enabled": s.ha_enabled, "key": s.ha_key}),
    );
    Value::Object(m)
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn mqtt_status_from(v: &Value) -> HubMqttStatus {
    HubMqttStatus {
        state: text(v.get("state")),
        msg: text(v.get("msg")),
        since: match v.get("since") {
            Some(Value::Number(n)) => n.to_string(),
            other => text(other),
        },
        queued: small(v.get("queued")).unwrap_or(0),
        dropped: v.get("dropped").and_then(Value::as_u64).unwrap_or(0),
        encrypted: flag(v.get("encrypted")),
        notice: text(v.get("notice")),
    }
}

// ── API keys ───────────────────────────────────────────────────────────────

/// An API key of the hub (`auth.list`'s rows of kind `api`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubApiKey {
    /// Its client id: what [`DspiCore::hub_api_key_revoke`] takes.
    pub cid: u32,
    /// Its name.
    pub name: String,
    /// `viewer`, `control` or `admin`.
    pub role: String,
    /// Kept for when the port speaks TLS; no effect yet.
    pub plaintext_ok: bool,
    /// Its REST session is open.
    pub online: bool,
    /// When it was minted (ISO-8601 UTC).
    pub created: String,
    /// When it was last used; "" for never.
    pub last_seen: String,
}

/// A key the hub just minted (`auth.mint`): [`HubApiKeyMinted::key`] is
/// shown in this answer only.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubApiKeyMinted {
    /// Its client id.
    pub cid: u32,
    /// Its name, as the hub cleaned it.
    pub name: String,
    /// Its role.
    pub role: String,
    /// As asked.
    pub plaintext_ok: bool,
    /// The key (43 characters): show it once; the hub keeps only its hash.
    #[serde(serialize_with = "reveal_ser")]
    pub key: Arc<HubSecret>,
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn key_from(v: &Value) -> HubApiKey {
    HubApiKey {
        cid: small(v.get("cid")).unwrap_or(0),
        name: text(v.get("name")),
        role: text(v.get("role")),
        plaintext_ok: flag(v.get("plaintext_ok")),
        online: flag(v.get("online")),
        created: text(v.get("created")),
        last_seen: text(v.get("last_seen")),
    }
}

// ── The calls ──────────────────────────────────────────────────────────────

#[cfg(not(feature = "link"))]
fn off_hub() -> Result<()> {
    Err(DspiError::Unsupported)
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The hub's alarms (`alarm.list`): the device's and the audio link's,
    /// those on first (newest first), then the cleared, at most 500.
    /// `serial` "" for every alarm, else only that device's; `state` ""
    /// (every one), `active` (raised or acknowledged) or `cleared`. Every
    /// role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `alarms` cap (nothing
    /// sent); [`DspiError::InvalidArgument`] for another `state`;
    /// [`DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_alarms(&self, serial: String, state: String) -> Result<Vec<HubAlarm>> {
        if !state.is_empty() && !ALARM_STATES.contains(&state.as_str()) {
            return Err(DspiError::InvalidArgument {
                msg: format!("state is active, cleared or all, not {state:?}"),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ALARMS)?;
            let mut f = Map::new();
            if !serial.is_empty() {
                f.insert("serial".into(), serial.into());
            }
            if !state.is_empty() {
                f.insert("state".into(), state.into());
            }
            let reply = self.hub_ask("alarm.list", f)?;
            Ok(reply
                .get("alarms")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(ROWS_MAX).map(alarm_from).collect())
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = serial;
            off_hub().map(|()| Vec::new())
        }
    }

    /// Acknowledge alarm `id` (`alarm.ack`): the alarm as it is now. A
    /// `device_restarted` or `schedule_failed` alarm is cleared by it.
    /// Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found` for an alarm that is not on;
    /// [`DspiError::Denied`] for a Viewer; [`DspiError::ReadOnly`]; as
    /// [`DspiCore::hub_alarms`].
    pub fn hub_alarm_ack(&self, id: String) -> Result<HubAlarm> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ALARMS)?;
            let mut f = Map::new();
            f.insert("alarm".into(), id.into());
            let reply = self.hub_ask("alarm.ack", f)?;
            reply
                .get("alarm")
                .filter(|a| a.is_object())
                .map(alarm_from)
                .ok_or_else(|| protocol("alarm.ack's answer has no alarm"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            off_hub().map(|()| HubAlarm::default())
        }
    }

    /// The alarm rules (`alarm.rules.get`), one row per code. Every role.
    /// Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_alarms`].
    pub fn hub_alarm_rules_get(&self) -> Result<Vec<HubAlarmRule>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ALARMS)?;
            let reply = self.hub_ask("alarm.rules.get", Map::new())?;
            Ok(rules_from(reply.get("rules").unwrap_or(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| Vec::new())
    }

    /// Change the alarm rules (`alarm.rules.set`): the codes given, each
    /// with `on` and the fields that are `Some` (the others keep the hub's
    /// values); a code left out keeps its rule. All or nothing. The rules
    /// now. Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a rule without a code or a
    /// code given twice (nothing sent), and the hub's `bad_request` naming a
    /// field out of range; [`DspiError::Denied`] below Admin;
    /// [`DspiError::ReadOnly`]; as [`DspiCore::hub_alarms`].
    pub fn hub_alarm_rules_set(&self, rules: Vec<HubAlarmRule>) -> Result<Vec<HubAlarmRule>> {
        let given = rules_to(&rules)?;
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ALARMS)?;
            let mut f = Map::new();
            f.insert("rules".into(), given);
            let reply = self.hub_ask("alarm.rules.set", f)?;
            Ok(rules_from(reply.get("rules").unwrap_or(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = given;
            off_hub().map(|()| Vec::new())
        }
    }

    /// The hub's schedule (`schedule.list`): the rules with their next and
    /// last runs, the time zone, the hub's local time and whether its clock
    /// is unset. Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `schedule` cap;
    /// [`DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_schedules(&self) -> Result<HubSchedules> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_SCHEDULE)?;
            let reply = self.hub_ask("schedule.list", Map::new())?;
            Ok(HubSchedules {
                rules: reply
                    .get("rules")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().take(ROWS_MAX).map(rule_from).collect())
                    .unwrap_or_default(),
                zone: text(reply.get("zone")),
                local_time: text(reply.get("local_time")),
                clock_unset: flag(reply.get("clock_unset")),
            })
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| HubSchedules::default())
    }

    /// Add a rule (`id` "") or replace rule `id` whole (`schedule.set`); its
    /// `next_run` and `last` are not sent. The rule as the hub lists it.
    /// Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for both `days` and `once`, a
    /// `param.set` without a value (nothing sent) and the hub's
    /// `bad_request`; [`DspiError::Link`] `not_found` (no rule `id`),
    /// `conflict` (256 rules); [`DspiError::Denied`] below Admin;
    /// [`DspiError::ReadOnly`]; as [`DspiCore::hub_schedules`].
    pub fn hub_schedule_set(&self, rule: HubScheduleRule) -> Result<HubScheduleRule> {
        let given = rule_to(&rule)?;
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_SCHEDULE)?;
            let mut f = Map::new();
            f.insert("rule".into(), given);
            let reply = self.hub_ask("schedule.set", f)?;
            reply
                .get("rule")
                .filter(|r| r.is_object())
                .map(rule_from)
                .ok_or_else(|| protocol("schedule.set's answer has no rule"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = given;
            off_hub().map(|()| HubScheduleRule::default())
        }
    }

    /// Delete rule `id` (`schedule.delete`). Admin of the hub. Slow: off the
    /// UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; as
    /// [`DspiCore::hub_schedule_set`].
    pub fn hub_schedule_delete(&self, id: String) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_SCHEDULE)?;
            let mut f = Map::new();
            f.insert("rule".into(), id.into());
            self.hub_ask("schedule.delete", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            off_hub()
        }
    }

    /// Run rule `id` now (`schedule.run_now`), whether it is enabled or not:
    /// the hub writes its targets as `Schedule: <name>`, and the run's
    /// results come back (one row per action and target; a failed row is not
    /// a failed call). Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`, `conflict` while the hub's
    /// clock is unset; as [`DspiCore::hub_schedule_set`].
    pub fn hub_schedule_run_now(&self, id: String) -> Result<HubScheduleRun> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_SCHEDULE)?;
            let mut f = Map::new();
            f.insert("rule".into(), id.into());
            let reply = self.hub_ask("schedule.run_now", f)?;
            Ok(run_from(&reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            off_hub().map(|()| HubScheduleRun::default())
        }
    }

    /// The notification targets (`outbound.get`), secrets as `set` or `not
    /// set`, with each one's queue and last try. Admin of the hub. Slow: off
    /// the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `outbound` cap;
    /// [`DspiError::Denied`] below Admin; [`DspiError::Disconnected`].
    pub fn hub_outbound_get(&self) -> Result<Vec<HubOutboundTarget>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_OUTBOUND)?;
            let reply = self.hub_ask("outbound.get", Map::new())?;
            Ok(targets_of(&reply))
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| Vec::new())
    }

    /// Replace the notification targets (`outbound.set`, the whole list, at
    /// most 16). A target with `secret` `new` gets a secret made by the hub:
    /// in [`HubOutboundSet::secrets`], this once. The list as
    /// [`DspiCore::hub_outbound_get`] gives can be sent back as it is. Admin
    /// of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with the hub's words naming the
    /// entry and field (nothing kept); as [`DspiCore::hub_outbound_get`];
    /// [`DspiError::ReadOnly`].
    pub fn hub_outbound_set(&self, targets: Vec<HubOutboundTarget>) -> Result<HubOutboundSet> {
        let list: Vec<Value> = targets.iter().map(target_to).collect();
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_OUTBOUND)?;
            let mut f = Map::new();
            f.insert("targets".into(), list.into());
            let reply = self.hub_ask("outbound.set", f)?;
            let secrets = reply
                .get("secrets")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .take(64)
                        .filter_map(|(id, s)| {
                            s.as_str().map(|s| HubOutboundSecret {
                                target: cut(id, 64),
                                secret: HubSecret::new(cut(s, 256)),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(HubOutboundSet {
                targets: targets_of(&reply),
                secrets,
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = list;
            off_hub().map(|()| HubOutboundSet::default())
        }
    }

    /// Send target `id` one delivery of kind `test` now (`outbound.test`),
    /// recorded as its last try. Admin of the hub. Slow (up to 10 s): off
    /// the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; as
    /// [`DspiCore::hub_outbound_set`].
    pub fn hub_outbound_test(&self, id: String) -> Result<HubOutboundTest> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_OUTBOUND)?;
            let mut f = Map::new();
            f.insert("target".into(), id.into());
            let reply = self.hub_ask("outbound.test", f)?;
            Ok(HubOutboundTest {
                delivered: flag(reply.get("delivered")),
                status: small(reply.get("status")),
                msg: text(reply.get("msg")),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            off_hub().map(|()| HubOutboundTest::default())
        }
    }

    /// The broker setting (`mqtt.get`): the values in use, the password and
    /// a client key as `set` or `not set`. Admin of the hub. Slow: off the UI
    /// thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `mqtt` cap;
    /// [`DspiError::Denied`] below Admin; [`DspiError::Disconnected`].
    pub fn hub_mqtt_get(&self) -> Result<HubMqttSettings> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_MQTT)?;
            let reply = self.hub_ask("mqtt.get", Map::new())?;
            Ok(mqtt_from(
                reply.get("settings").unwrap_or(&Value::Null),
                reply.get("notice"),
            ))
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| HubMqttSettings::default())
    }

    /// Change the broker setting (`mqtt.set`): every field as given (what
    /// [`DspiCore::hub_mqtt_get`] answered can be sent back as it is; see
    /// [`HubMqttSettings`] for "" and the password). Any change but Home
    /// Assistant's ends the connection and starts a new one. The setting
    /// now. Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with the hub's words naming the
    /// field (nothing changed); as [`DspiCore::hub_mqtt_get`];
    /// [`DspiError::ReadOnly`].
    pub fn hub_mqtt_set(&self, settings: HubMqttSettings) -> Result<HubMqttSettings> {
        let given = mqtt_to(&settings);
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_MQTT)?;
            let mut f = Map::new();
            f.insert("settings".into(), given);
            let reply = self.hub_ask("mqtt.set", f)?;
            Ok(mqtt_from(
                reply.get("settings").unwrap_or(&Value::Null),
                reply.get("notice"),
            ))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = given;
            off_hub().map(|()| HubMqttSettings::default())
        }
    }

    /// The broker connection now (`mqtt.status`). Admin of the hub. Slow: off
    /// the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_mqtt_get`].
    pub fn hub_mqtt_status(&self) -> Result<HubMqttStatus> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_MQTT)?;
            let reply = self.hub_ask("mqtt.status", Map::new())?;
            Ok(mqtt_status_from(&reply))
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| HubMqttStatus::default())
    }

    /// The hub's API keys (`auth.list`'s rows of kind `api`; the consoles'
    /// pairings are left out). Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `api_keys` cap;
    /// [`DspiError::Denied`] below Admin; [`DspiError::Disconnected`].
    pub fn hub_api_keys(&self) -> Result<Vec<HubApiKey>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_API_KEYS)?;
            let reply = self.hub_ask("auth.list", Map::new())?;
            Ok(reply
                .get("clients")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(ROWS_MAX)
                        .filter(|c| c.get("kind").and_then(Value::as_str) == Some("api"))
                        .map(key_from)
                        .collect()
                })
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        off_hub().map(|()| Vec::new())
    }

    /// Mint an API key `name` with `role` (`viewer`, `control`, `admin`;
    /// never above this session's) for REST and Home Assistant
    /// (`auth.mint`). The key is in this answer only: show it once. Admin of
    /// the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another role (nothing
    /// sent) or the hub's `bad_request`; [`DspiError::Denied`] below Admin or
    /// for a role above this session's; [`DspiError::Link`] `conflict` with
    /// 64 clients and keys; [`DspiError::ReadOnly`]; as
    /// [`DspiCore::hub_api_keys`].
    pub fn hub_api_key_mint(
        &self,
        name: String,
        role: String,
        plaintext_ok: bool,
    ) -> Result<HubApiKeyMinted> {
        if !["viewer", "control", "admin"].contains(&role.as_str()) {
            return Err(DspiError::InvalidArgument {
                msg: format!("role is viewer, control or admin, not {role:?}"),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_API_KEYS)?;
            let mut f = Map::new();
            f.insert("name".into(), name.into());
            f.insert("role".into(), role.into());
            f.insert("plaintext_ok".into(), plaintext_ok.into());
            let reply = self.hub_ask("auth.mint", f)?;
            let key = reply
                .get("key")
                .and_then(Value::as_str)
                .map(|k| cut(k, 256))
                .filter(|k| !k.is_empty())
                .ok_or_else(|| protocol("auth.mint's answer has no key"))?;
            Ok(HubApiKeyMinted {
                cid: small(reply.get("cid")).unwrap_or(0),
                name: text(reply.get("name")),
                role: text(reply.get("role")),
                plaintext_ok: flag(reply.get("plaintext_ok")),
                key: HubSecret::new(key),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (name, plaintext_ok);
            off_hub().map(|()| HubApiKeyMinted::default())
        }
    }

    /// Revoke API key `cid` (`auth.revoke`): its REST session ends. Admin of
    /// the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a `cid` that is not an API
    /// key's here (nothing sent, so a console's pairing is never revoked by
    /// this call) or that the hub does not know; as
    /// [`DspiCore::hub_api_key_mint`].
    pub fn hub_api_key_revoke(&self, cid: u32) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_API_KEYS)?;
            if !self.hub_api_keys()?.iter().any(|k| k.cid == cid) {
                return Err(DspiError::InvalidArgument {
                    msg: format!("no API key {cid} on this hub"),
                });
            }
            let mut f = Map::new();
            f.insert("cid".into(), cid.into());
            self.hub_ask("auth.revoke", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = cid;
            off_hub()
        }
    }
}

#[cfg(feature = "link")]
fn targets_of(reply: &Value) -> Vec<HubOutboundTarget> {
    reply
        .get("targets")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(64).map(target_from).collect())
        .unwrap_or_default()
}

#[cfg(feature = "link")]
fn protocol(msg: &str) -> DspiError {
    DspiError::Link {
        code: "protocol".into(),
        msg: msg.into(),
    }
}

#[cfg(test)]
#[path = "tests_wp_p2cc2.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2cc2;
