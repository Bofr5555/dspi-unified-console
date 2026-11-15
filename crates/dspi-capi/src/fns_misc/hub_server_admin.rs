//! C functions: the hub's admin families (WP P2CC2), over
//! `dspi_core::hub_server::admin` (`docs/integrator/hub-api.md` sections 7,
//! 8, 12, 13.1, 14.1): alarms, schedules, notifications out, MQTT and API
//! keys. Replies as in the parent module's docs: JSON, the records' fields
//! camelCase beside `"ok": true`, a list as `value`. A record argument is its
//! JSON text, in the same camelCase (a field left out is its default).
//!
//! Without the family's cap (or off a hub) a call fails with `code`
//! `unsupported` and sends nothing; the hub's refusals as in
//! `hub_server.rs`. The hub's `alarm.changed` and `schedule.ran` come through
//! `dspi_hub_server_events` and its listener (`alarm`, `run`). A minted key
//! and a webhook's new secret are in the reply of the call that made them,
//! once. Every call but `dspi_hub_alarm_notice` is a round trip: off the UI
//! thread.

use std::ffi::c_char;

use dspi_core::{DspiError, HubAlarm, Notice, Result};
use serde::de::DeserializeOwned;

use super::{arg, done_reply, json_reply, optional_text, text};
use crate::guard;

/// `json` (a record's JSON text) as `T`; `InvalidArgument` naming `name` when
/// it does not read.
fn record<T: DeserializeOwned>(json: *const std::ffi::c_char, name: &str) -> Result<T> {
    let text = arg(json, name)?;
    serde_json::from_str(text).map_err(|e| DspiError::InvalidArgument {
        msg: format!("{name}: {e}"),
    })
}

/// The hub's alarms of `serial` (null or "": every alarm) in `state` (null
/// or "": every one; `active`, `cleared`): JSON `{ok, value: [{id, source,
/// code, subject, serial, severity, message, state, raisedAt, clearedAt,
/// acknowledgedBy, count, reason, noticeKey}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_alarms(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    state: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(optional_text(serial, "serial").and_then(|s| {
            optional_text(state, "state")
                .and_then(|st| c.hub_alarms(s.unwrap_or_default(), st.unwrap_or_default()))
        }))
    })
}

/// Acknowledge alarm `id` (Control, Admin): the alarm's JSON as it is now.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_alarm_ack(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_alarm_ack(i.to_owned())))
    })
}

/// The alarm rules: JSON `{ok, value: [{code, on, afterS, perMinute,
/// muteOnStorm, aboveC, abovePct, forS}]}` (a field the code lacks is null).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_alarm_rules_get(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_alarm_rules_get()))
}

/// Change the alarm rules (Admin): `rules_json` a JSON list of rules as
/// `dspi_hub_alarm_rules_get` gives (null fields keep the hub's values). The
/// rules now, as there.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_alarm_rules_set(
    core: *mut crate::FfiCore,
    rules_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(record(rules_json, "rules").and_then(|r| c.hub_alarm_rules_set(r)))
    })
}

/// What to tell the user about an alarm: `alarm_json` one alarm as the
/// calls above give it, `time` the client's rendering of its `raisedAt`
/// (null or "": the hub's ISO time). JSON `{ok, key, kind, title, reason,
/// action, detail}`. No I/O; no core.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_alarm_notice(
    alarm_json: *const c_char,
    time: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let reply: Result<Notice> = record::<HubAlarm>(alarm_json, "alarm").and_then(|a| {
            optional_text(time, "time")
                .map(|t| dspi_core::hub_alarm_notice(a, t.unwrap_or_default()))
        });
        crate::fns_device::copy_text(&json_reply(reply), buf, len)
    })
}

/// The hub's schedule: JSON `{ok, rules: [{id, name, enabled, days, time,
/// once, actions: [{t, slot, name, snapshot, scene, path, idx, value,
/// flag}], targets, nextRun, last}], zone, localTime, clockUnset}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_schedules(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_schedules()))
}

/// Add (`id` "") or replace a schedule rule (Admin): `rule_json` one rule
/// as `dspi_hub_schedules` lists it. The rule as the hub lists it.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_schedule_set(
    core: *mut crate::FfiCore,
    rule_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(record(rule_json, "rule").and_then(|r| c.hub_schedule_set(r)))
    })
}

/// Delete schedule rule `id` (Admin): `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_schedule_delete(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(id, "id").and_then(|i| c.hub_schedule_delete(i.to_owned())))
    })
}

/// Run schedule rule `id` now (Admin; it writes its targets): JSON `{ok,
/// rule, name, at, by, results: [{serial, action, ok, code, msg}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_schedule_run_now(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_schedule_run_now(i.to_owned())))
    })
}

/// The notification targets (Admin): JSON `{ok, value: [{id, name, kind,
/// url, enabled, kinds, secret, plaintext, caPem, pinSha256, queued,
/// dropped, last, encrypted, trust, notice}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_outbound_get(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_outbound_get()))
}

/// Replace the notification targets (Admin): `targets_json` the whole list
/// as `dspi_hub_outbound_get` gives (`secret` `new` for a new secret). JSON
/// `{ok, targets, secrets: [{target, secret}]}`: the new secrets, this once.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_outbound_set(
    core: *mut crate::FfiCore,
    targets_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(record(targets_json, "targets").and_then(|t| c.hub_outbound_set(t)))
    })
}

/// One test delivery to target `id` now (Admin): JSON `{ok, delivered,
/// status, msg}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_outbound_test(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_outbound_test(i.to_owned())))
    })
}

/// The broker setting (Admin): JSON `{ok, enabled, broker, port, clientId,
/// username, password, baseTopic, tls: {mode, caPem, pinSha256,
/// clientCertPem, clientKeyPem, clientCertFile, clientKeyFile,
/// allowPlaintextPassword}, haEnabled, haKey, notice}` (the password and the
/// client key as `set` or `not set`).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_mqtt_get(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_mqtt_get()))
}

/// Change the broker setting (Admin): `settings_json` as `dspi_hub_mqtt_get`
/// gives it (`password` the password to set it, `set` or "" to keep it). The
/// setting now, as there.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_mqtt_set(
    core: *mut crate::FfiCore,
    settings_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(record(settings_json, "settings").and_then(|s| c.hub_mqtt_set(s)))
    })
}

/// The broker connection now (Admin): JSON `{ok, state, msg, since,
/// queued, dropped, encrypted, notice}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_mqtt_status(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_mqtt_status()))
}

/// The hub's API keys (Admin): JSON `{ok, value: [{cid, name, role,
/// plaintextOk, online, created, lastSeen}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_api_keys(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_api_keys()))
}

/// Mint an API key `name` with `role` (`viewer`, `control`, `admin`)
/// (Admin): JSON `{ok, cid, name, role, plaintextOk, key}`: the key, this
/// once.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_api_key_mint(
    core: *mut crate::FfiCore,
    name: *const c_char,
    role: *const c_char,
    plaintext_ok: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(name, "name").and_then(|n| {
            arg(role, "role")
                .and_then(|r| c.hub_api_key_mint(n.to_owned(), r.to_owned(), plaintext_ok))
        }))
    })
}

/// Revoke API key `cid` (Admin): `{ok}` or a failure (`invalid` for a cid
/// that is not an API key's).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_api_key_revoke(
    core: *mut crate::FfiCore,
    cid: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| done_reply(c.hub_api_key_revoke(cid)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p2cc2 {
    use std::ffi::{CString, c_char};

    use serde_json::{Value, json};

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    /// The same words as the core's own calls, through C.
    #[test]
    fn wp_p2cc2_the_admin_calls_through_c() {
        let h = dspi_core_new();
        let caps = reply(|b, l| super::super::hub_server::dspi_hub_caps(h, b, l));
        assert_eq!(caps["alarms"], false, "{caps}");
        assert_eq!(caps["apiKeys"], false, "{caps}");
        let id = CString::new("r1").unwrap();
        let rules = CString::new(r#"[{"code": "cpu", "on": true, "abovePct": 80}]"#).unwrap();
        let rule = CString::new(r#"{"name": "x", "days": ["mon"], "time": "10:00"}"#).unwrap();
        let targets = CString::new("[]").unwrap();
        let settings = CString::new(r#"{"enabled": false}"#).unwrap();
        let name = CString::new("Script").unwrap();
        let role = CString::new("viewer").unwrap();
        for r in [
            reply(|b, l| dspi_hub_alarms(h, std::ptr::null(), std::ptr::null(), b, l)),
            reply(|b, l| dspi_hub_alarm_ack(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_alarm_rules_get(h, b, l)),
            reply(|b, l| dspi_hub_alarm_rules_set(h, rules.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_schedules(h, b, l)),
            reply(|b, l| dspi_hub_schedule_set(h, rule.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_schedule_delete(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_schedule_run_now(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_outbound_get(h, b, l)),
            reply(|b, l| dspi_hub_outbound_set(h, targets.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_outbound_test(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_mqtt_get(h, b, l)),
            reply(|b, l| dspi_hub_mqtt_set(h, settings.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_mqtt_status(h, b, l)),
            reply(|b, l| dspi_hub_api_keys(h, b, l)),
            reply(|b, l| dspi_hub_api_key_mint(h, name.as_ptr(), role.as_ptr(), false, b, l)),
            reply(|b, l| dspi_hub_api_key_revoke(h, 3, b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "unsupported", "{r}");
            assert_eq!(r["noticeKey"], "unsupported", "{r}");
        }
        // A record that does not read is the caller's error, nothing sent.
        let bad = CString::new("{not json").unwrap();
        let r = reply(|b, l| dspi_hub_schedule_set(h, bad.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = reply(|b, l| dspi_hub_alarms(h, std::ptr::null(), id.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "a state the hub has not: {r}");
        dspi_core_free(h);

        // The notice: the core's words, filled.
        let alarm = CString::new(
            json!({"code": "device_restarted", "reason": "watchdog",
                   "message": "m", "raisedAt": "2026-11-15T09:30:00Z"})
            .to_string(),
        )
        .unwrap();
        let time = CString::new("10:30").unwrap();
        let n = reply(|b, l| dspi_hub_alarm_notice(alarm.as_ptr(), time.as_ptr(), b, l));
        assert_eq!(n["ok"], true, "{n}");
        assert_eq!(n["key"], "alarm.device_restarted");
        assert_eq!(n["title"], "Restarted at 10:30 (watchdog)");
        assert_eq!(n["detail"], "m");
        let core_says = dspi_core::hub_alarm_notice(
            serde_json::from_str(alarm.to_str().unwrap()).unwrap(),
            "10:30".into(),
        );
        assert_eq!(n["reason"], core_says.reason.as_str());
    }
}
