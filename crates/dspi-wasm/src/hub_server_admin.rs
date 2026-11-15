//! The hub's admin families on the web (WP P2CC2), over
//! `dspi_core::hub_server::admin` (`docs/integrator/hub-api.md` sections 7,
//! 8, 12, 13.1, 14.1), through this session's DSPi Link. Records in and out
//! are the core's, camelCase:
//!
//! - `hubAlarms {serial?, state?}`, `hubAlarmAck {id}`, `hubAlarmRulesGet`,
//!   `hubAlarmRulesSet {rules}`, `hubAlarmNotice {alarm, time?}` (no I/O).
//! - `hubSchedules`, `hubScheduleSet {rule}`, `hubScheduleDelete {id}`,
//!   `hubScheduleRunNow {id}`.
//! - `hubOutboundGet`, `hubOutboundSet {targets}` (the new secrets in
//!   `secrets`, this once), `hubOutboundTest {id}`.
//! - `hubMqttGet`, `hubMqttSet {settings}`, `hubMqttStatus`.
//! - `hubApiKeys`, `hubApiKeyMint {name, role, plaintextOk?}` (the key, this
//!   once), `hubApiKeyRevoke {cid}`.
//!
//! The hub's `alarm.changed` and `schedule.ran` come with `hubServerEvents`
//! (`alarm`, `run`). Without the family's cap every call answers
//! `unsupported` and sends nothing.

use dspi_core::{
    DspiCore, HubAlarm, HubAlarmRule, HubMqttSettings, HubOutboundTarget, HubScheduleRule,
};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    // No parameters is every field's default.
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v
    };
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Alarms {
    serial: String,
    state: String,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
struct Rules {
    rules: Vec<HubAlarmRule>,
}

#[derive(Deserialize)]
struct AlarmNotice {
    alarm: HubAlarm,
    #[serde(default)]
    time: String,
}

#[derive(Deserialize)]
struct Rule {
    rule: HubScheduleRule,
}

#[derive(Deserialize)]
struct Targets {
    targets: Vec<HubOutboundTarget>,
}

#[derive(Deserialize)]
struct Settings {
    settings: HubMqttSettings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mint {
    name: String,
    role: String,
    #[serde(default)]
    plaintext_ok: bool,
}

#[derive(Deserialize)]
struct Cid {
    cid: u32,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "hubAlarms" => params::<Alarms>(p).and_then(|x| record(core.hub_alarms(x.serial, x.state))),
        "hubAlarmAck" => params::<Id>(p).and_then(|x| record(core.hub_alarm_ack(x.id))),
        "hubAlarmRulesGet" => record(core.hub_alarm_rules_get()),
        "hubAlarmRulesSet" => {
            params::<Rules>(p).and_then(|x| record(core.hub_alarm_rules_set(x.rules)))
        }
        "hubAlarmNotice" => {
            params::<AlarmNotice>(p).map(|x| tidy(&dspi_core::hub_alarm_notice(x.alarm, x.time)))
        }
        "hubSchedules" => record(core.hub_schedules()),
        "hubScheduleSet" => params::<Rule>(p).and_then(|x| record(core.hub_schedule_set(x.rule))),
        "hubScheduleDelete" => params::<Id>(p)
            .and_then(|x| record(core.hub_schedule_delete(x.id).map(|()| Value::Null))),
        "hubScheduleRunNow" => {
            params::<Id>(p).and_then(|x| record(core.hub_schedule_run_now(x.id)))
        }
        "hubOutboundGet" => record(core.hub_outbound_get()),
        "hubOutboundSet" => {
            params::<Targets>(p).and_then(|x| record(core.hub_outbound_set(x.targets)))
        }
        "hubOutboundTest" => params::<Id>(p).and_then(|x| record(core.hub_outbound_test(x.id))),
        "hubMqttGet" => record(core.hub_mqtt_get()),
        "hubMqttSet" => params::<Settings>(p).and_then(|x| record(core.hub_mqtt_set(x.settings))),
        "hubMqttStatus" => record(core.hub_mqtt_status()),
        "hubApiKeys" => record(core.hub_api_keys()),
        "hubApiKeyMint" => params::<Mint>(p)
            .and_then(|x| record(core.hub_api_key_mint(x.name, x.role, x.plaintext_ok))),
        "hubApiKeyRevoke" => params::<Cid>(p)
            .and_then(|x| record(core.hub_api_key_revoke(x.cid).map(|()| Value::Null))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p2cc2 {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    /// The same words as the core's own calls, on the web.
    #[test]
    fn wp_p2cc2_the_admin_calls_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let caps = run(h, "hubCaps", Value::Null);
        assert_eq!(caps["value"]["alarms"], false, "{caps}");
        assert_eq!(caps["value"]["schedule"], false, "{caps}");
        for (m, p) in [
            ("hubAlarms", Value::Null),
            ("hubAlarmAck", json!({"id": "device:1"})),
            ("hubAlarmRulesGet", Value::Null),
            (
                "hubAlarmRulesSet",
                json!({"rules": [{"code": "cpu", "on": false}]}),
            ),
            ("hubSchedules", Value::Null),
            (
                "hubScheduleSet",
                json!({"rule": {"name": "x", "days": ["mon"], "time": "10:00"}}),
            ),
            ("hubScheduleDelete", json!({"id": "r1"})),
            ("hubScheduleRunNow", json!({"id": "r1"})),
            ("hubOutboundGet", Value::Null),
            ("hubOutboundSet", json!({"targets": []})),
            ("hubOutboundTest", json!({"id": "t1"})),
            ("hubMqttGet", Value::Null),
            ("hubMqttSet", json!({"settings": {"enabled": false}})),
            ("hubMqttStatus", Value::Null),
            ("hubApiKeys", Value::Null),
            ("hubApiKeyMint", json!({"name": "k", "role": "viewer"})),
            ("hubApiKeyRevoke", json!({"cid": 3})),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["code"], "unsupported", "{m}: {r}");
            assert_eq!(r["noticeKey"], "unsupported", "{m}: {r}");
        }
        let r = run(
            h,
            "hubAlarmNotice",
            json!({"alarm": {"code": "device_restarted", "reason": "glitch",
                             "raisedAt": "2026-11-15T09:30:00Z", "message": "m"},
                   "time": "10:30"}),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["title"], "Restarted at 10:30 (glitch)", "{r}");
        assert_eq!(r["value"]["key"], "alarm.device_restarted", "{r}");
    }

    /// WP P2DFIXB: a secret is an object in the core, and still the plain
    /// string in the web's answer.
    #[test]
    fn wp_p2dfixb_web_answers_carry_the_secret_as_a_string() {
        let minted = dspi_core::HubApiKeyMinted {
            cid: 6,
            name: "Script".into(),
            role: "viewer".into(),
            plaintext_ok: false,
            key: dspi_core::HubSecret::new("K".repeat(43)),
        };
        let v = crate::calls::tidy(&minted);
        assert_eq!(v["key"], "K".repeat(43), "{v}");
        let set = dspi_core::HubOutboundSet {
            targets: Vec::new(),
            secrets: vec![dspi_core::HubOutboundSecret {
                target: "t1".into(),
                secret: dspi_core::HubSecret::new("ab".repeat(32)),
            }],
        };
        let v = crate::calls::tidy(&set);
        assert_eq!(
            v["secrets"][0],
            json!({"target": "t1", "secret": "ab".repeat(32)})
        );
    }
}
