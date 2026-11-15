//! WP P2CC2: the typed calls for the hub's admin families (alarms, schedules,
//! notifications out, MQTT, API keys), against the fake hub and without one,
//! and the alarm notice rows. The same calls against the real hub are in
//! `crates/dspi-hub/tests/wp_p2cc2_core_calls.rs`.

use serde_json::json;

use super::*;
use crate::notice_text::has_row;

#[test]
fn wp_p2cc2_every_alarm_code_has_a_notice_row() {
    for code in ALARM_CODES {
        let key = format!("{NOTICE_ALARM_FAMILY}.{code}");
        assert!(has_row(&key), "{key}: no notice row");
        let alarm = alarm_from(&json!({"id": "device:1", "code": code, "message": "words",
            "raised_at": "2026-11-15T09:30:00Z"}));
        assert_eq!(alarm.notice_key, key);
        let n = hub_alarm_notice(alarm, "10:30".into());
        assert_eq!(n.key, key, "{code}");
        assert_eq!(n.detail, "words");
        for s in [&n.title, &n.reason, &n.action] {
            assert!(!s.is_empty(), "{code}");
            assert!(!s.contains('{') && !s.contains('}'), "{code}: {s}");
            // Shared wording: no host-device nouns.
            for noun in ["computer", "Mac", "PC", "phone", "iPad", "tablet"] {
                assert!(!s.contains(noun), "{code}: {noun} in {s}");
            }
        }
    }
    // A code this build does not know: its own row, never a bare fallback.
    let a = alarm_from(&json!({"code": "something_new", "message": "m"}));
    assert_eq!(a.notice_key, NOTICE_ALARM_UNKNOWN);
    assert!(has_row(NOTICE_ALARM_UNKNOWN));
    assert_eq!(hub_alarm_notice(a, String::new()).key, NOTICE_ALARM_UNKNOWN);
}

#[test]
fn wp_p2cc2_a_restart_notice_says_when_and_why() {
    let a = alarm_from(
        &json!({"id": "device:3", "source": "device", "code": "device_restarted",
        "subject": "S1", "serial": "S1", "severity": "warning",
        "message": "Restarted at 2026-11-15T09:30:00Z (watchdog: the watchdog reset it)",
        "state": "raised", "raised_at": "2026-11-15T09:30:05Z", "count": 0}),
    );
    assert_eq!(a.reason, "watchdog");
    assert_eq!(a.serial, "S1");
    let n = hub_alarm_notice(a.clone(), "10:30".into());
    assert_eq!(n.title, "Restarted at 10:30 (watchdog)");
    assert_eq!(n.kind, crate::NoticeKind::Warning);
    // No time given: the hub's.
    let n = hub_alarm_notice(a, String::new());
    assert_eq!(n.title, "Restarted at 2026-11-15T09:30:05Z (watchdog)");
    let b = alarm_from(&json!({"code": "device_restarted",
        "message": "Restarted at 2026-11-15T09:30:00Z (brownout: the supply dipped)"}));
    assert_eq!(b.reason, "brownout");
    assert!(
        hub_alarm_notice(b, "t".into())
            .title
            .ends_with("(brown-out)")
    );
    // A message without a reason: none, and the notice says so.
    let c = alarm_from(&json!({"code": "device_restarted", "message": "Restarted"}));
    assert_eq!(c.reason, "");
    assert!(
        hub_alarm_notice(c, "t".into())
            .title
            .contains("unknown cause")
    );
    // Only a restart reads a reason.
    let d = alarm_from(&json!({"code": "cpu", "message": "CPU 95 % (x: y)"}));
    assert_eq!(d.reason, "");
}

#[test]
fn wp_p2cc2_the_shapes_on_the_wire() {
    // Rules: only the fields given; the hub's order kept on the way back.
    let rules = vec![
        HubAlarmRule {
            code: "clip".into(),
            on: true,
            per_minute: Some(20),
            mute_on_storm: Some(true),
            ..HubAlarmRule::default()
        },
        HubAlarmRule {
            code: "temperature".into(),
            on: false,
            above_c: Some(65.5),
            ..HubAlarmRule::default()
        },
    ];
    let v = rules_to(&rules).unwrap();
    assert_eq!(
        v,
        json!({"clip": {"on": true, "per_minute": 20, "mute_on_storm": true},
               "temperature": {"on": false, "above_c": 65.5}})
    );
    let back = rules_from(&v);
    assert_eq!(back, rules);
    let twice = vec![rules[0].clone(), rules[0].clone()];
    assert!(matches!(
        rules_to(&twice),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        rules_to(&[HubAlarmRule::default()]),
        Err(DspiError::InvalidArgument { .. })
    ));

    // A schedule rule: the hub's read-only fields are not sent; a mute is a
    // bool, a volume a number.
    let hub_rule = json!({"id": "r1", "name": "Doors open", "enabled": true,
        "when": {"days": ["mon", "fri"], "time": "18:30"},
        "actions": [{"t": "preset.load", "name": "Live"},
                    {"t": "param.set", "path": "vol.master", "value": -12.0},
                    {"t": "param.set", "path": "vol.mute", "value": true}],
        "targets": ["S1"], "next_run": "2026-11-16T17:30:00Z",
        "last": {"at": "2026-11-13T17:30:00Z", "ok": false}});
    let r = rule_from(&hub_rule);
    assert_eq!((r.days.len(), r.time.as_str()), (2, "18:30"));
    assert_eq!(r.actions[1].value, Some(-12.0));
    assert_eq!(r.actions[2].flag, Some(true));
    assert_eq!(r.last.as_ref().map(|l| l.ok), Some(false));
    let sent = rule_to(&r).unwrap();
    assert_eq!(
        sent,
        json!({"id": "r1", "name": "Doors open", "enabled": true,
            "when": {"days": ["mon", "fri"], "time": "18:30"},
            "actions": [{"t": "preset.load", "name": "Live"},
                        {"t": "param.set", "path": "vol.master", "value": -12.0},
                        {"t": "param.set", "path": "vol.mute", "value": true}],
            "targets": ["S1"]})
    );
    let mut once = r.clone();
    once.id = String::new();
    once.once = "2026-12-24T17:00".into();
    assert!(matches!(
        rule_to(&once),
        Err(DspiError::InvalidArgument { .. })
    ));
    once.days.clear();
    let sent = rule_to(&once).unwrap();
    assert_eq!(sent["when"], json!({"once": "2026-12-24T17:00"}));
    assert!(sent.get("id").is_none(), "a new rule: {sent}");
    let mut no_value = r;
    no_value.actions[1].value = None;
    assert!(matches!(
        rule_to(&no_value),
        Err(DspiError::InvalidArgument { .. })
    ));

    // A target: the settable fields; "" secret keeps it.
    let t = target_from(&json!({"id": "t1", "name": "Ops", "kind": "webhook",
        "url": "https://ops.example/hook", "enabled": true, "kinds": ["alarm.raised"],
        "secret": "set", "queued": 2, "dropped": 1, "plaintext": false,
        "last": {"at": "2026-11-15T10:00:00Z", "ok": false, "status": 500, "msg": "server error"},
        "encrypted": true, "trust": "webpki"}));
    assert_eq!(t.last.as_ref().and_then(|l| l.status), Some(500));
    assert_eq!(
        target_to(&t),
        json!({"id": "t1", "name": "Ops", "kind": "webhook", "url": "https://ops.example/hook",
               "enabled": true, "kinds": ["alarm.raised"], "secret": "set", "plaintext": false})
    );

    // MQTT: "" is the default or none; the password only when given.
    let s = mqtt_from(
        &json!({"enabled": true, "broker": "10.0.0.2", "port": 1883, "client_id": "dspiabc",
                "username": "hub", "password": "set", "base_topic": "dspi/abc",
                "tls": {"mode": "off"}, "ha": {"enabled": false, "key": null}}),
        None,
    );
    let mut patch = s.clone();
    patch.password = String::new();
    patch.client_id = String::new();
    let v = mqtt_to(&patch);
    assert!(v.get("password").is_none(), "{v}");
    assert_eq!(v["client_id"], Value::Null);
    assert_eq!(v["tls"]["mode"], "off");
    assert_eq!(v["tls"]["ca_pem"], Value::Null);
    assert_eq!(v["ha"], json!({"enabled": false, "key": null}));
    assert_eq!(mqtt_to(&s)["password"], "set");
}

#[test]
fn wp_p2cc2_secrets_never_show_in_debug() {
    let minted = HubApiKeyMinted {
        cid: 7,
        name: "HA".into(),
        role: "control".into(),
        plaintext_ok: false,
        key: "k3y-only-once-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
    };
    let secret = HubOutboundSecret {
        target: "t1".into(),
        secret: "5ec2e7".repeat(10),
    };
    let mqtt = HubMqttSettings {
        password: "hunter2-broker".into(),
        tls: HubMqttTls {
            client_key_pem: "-----BEGIN PRIVATE KEY-----MIIE".into(),
            ..HubMqttTls::default()
        },
        ..HubMqttSettings::default()
    };
    let set = HubOutboundSet {
        targets: Vec::new(),
        secrets: vec![secret.clone()],
    };
    let all = format!("{minted:?} {secret:?} {mqtt:?} {set:?}");
    for leaked in [
        "k3y-only-once",
        "5ec2e7",
        "hunter2-broker",
        "BEGIN PRIVATE KEY",
    ] {
        assert!(!all.contains(leaked), "{leaked} in {all}");
    }
    assert!(all.contains("\"set\""), "{all}");
    // The answer itself (C, wasm) has it, once.
    assert_eq!(
        serde_json::to_value(&minted).unwrap()["key"],
        minted.key.as_str()
    );
}

#[test]
fn wp_p2cc2_off_a_hub_every_call_is_unsupported() {
    let core = DspiCore::new();
    let caps = core.hub_caps();
    assert!(!caps.alarms && !caps.schedule && !caps.outbound && !caps.mqtt && !caps.api_keys);
    let u = DspiError::Unsupported;
    assert_eq!(
        core.hub_alarms(String::new(), String::new()).unwrap_err(),
        u
    );
    assert_eq!(core.hub_alarm_ack("device:1".into()).unwrap_err(), u);
    assert_eq!(core.hub_alarm_rules_get().unwrap_err(), u);
    assert_eq!(core.hub_alarm_rules_set(Vec::new()).unwrap_err(), u);
    assert_eq!(core.hub_schedules().unwrap_err(), u);
    assert_eq!(core.hub_schedule_delete("r1".into()).unwrap_err(), u);
    assert_eq!(core.hub_schedule_run_now("r1".into()).unwrap_err(), u);
    assert_eq!(core.hub_outbound_get().unwrap_err(), u);
    assert_eq!(core.hub_outbound_set(Vec::new()).unwrap_err(), u);
    assert_eq!(core.hub_outbound_test("t1".into()).unwrap_err(), u);
    assert_eq!(core.hub_mqtt_get().unwrap_err(), u);
    assert_eq!(
        core.hub_mqtt_set(HubMqttSettings::default()).unwrap_err(),
        u
    );
    assert_eq!(core.hub_mqtt_status().unwrap_err(), u);
    assert_eq!(core.hub_api_keys().unwrap_err(), u);
    assert_eq!(
        core.hub_api_key_mint("k".into(), "viewer".into(), false)
            .unwrap_err(),
        u
    );
    assert_eq!(core.hub_api_key_revoke(3).unwrap_err(), u);
    // The local checks come first, whatever the hub.
    assert!(matches!(
        core.hub_alarms(String::new(), "on".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.hub_api_key_mint("k".into(), "root".into(), false),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[cfg(feature = "link")]
mod through_the_fake_hub {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use dspi_link::fakehub::{FakeHub, HubConfig, JsonResponder};

    use super::*;
    use crate::{HubServerEvent, HubServerListener};

    const SERIAL: &str = "0123456789ABCDEF";
    const FAMILY_CAPS: [&str; 5] = [
        CAP_ALARMS,
        CAP_SCHEDULE,
        CAP_OUTBOUND,
        CAP_MQTT,
        CAP_API_KEYS,
    ];
    const FAMILIES: [&str; 5] = ["alarm.", "schedule.", "outbound.", "mqtt.", "auth."];

    fn hub(with_caps: bool) -> FakeHub {
        let mut cfg = HubConfig::default();
        if with_caps {
            cfg.caps.extend(FAMILY_CAPS.iter().map(|c| (*c).to_owned()));
        }
        FakeHub::start(cfg).unwrap()
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn bound(hub: &FakeHub) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.link_connect(
            hub.url(),
            "wp-p2cc2".into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        wait_until("the device list", || !core.link_devices().is_empty());
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core.ensure_probed();
        core
    }

    /// The requests of these families the hub got (`auth.pair`/`auth.token`
    /// of the connection left out).
    fn sent(hub: &FakeHub) -> Vec<Value> {
        hub.json_requests()
            .into_iter()
            .filter(|r| {
                let t = r["t"].as_str().unwrap_or("");
                FAMILIES.iter().any(|p| t.starts_with(p))
                    && !["auth.pair", "auth.token", "auth.hello"].contains(&t)
            })
            .collect()
    }

    fn last(hub: &FakeHub, t: &str) -> Value {
        sent(hub).into_iter().rfind(|r| r["t"] == t).unwrap()
    }

    fn alarm(id: &str, code: &str, state: &str) -> Value {
        json!({"id": id, "source": "device", "code": code, "subject": SERIAL, "serial": SERIAL,
               "severity": "warning", "message": "Restarted at 2026-11-15T09:30:00Z (glitch: a glitch)",
               "state": state, "raised_at": "2026-11-15T09:30:01Z", "count": 1})
    }

    /// Answers like the real hub's, from `hub-api.md`.
    fn answers() -> JsonResponder {
        Arc::new(move |r: &Value| {
            let t = r["t"].as_str()?;
            Some(match t {
                "alarm.list" => json!({"v": 1, "alarms": [
                    alarm("device:1", "device_restarted", "raised"),
                    {"id": "alink:4", "source": "alink", "code": "alink_lost", "subject": "unit 2",
                     "severity": "critical", "message": "Unit 2 is searching", "state": "acknowledged",
                     "raised_at": "2026-11-15T09:00:00Z", "acknowledged_by": "Phone", "count": 3}]}),
                "alarm.ack" => {
                    if r["alarm"] == "device:9" {
                        json!({"code": "not_found", "msg": "no such alarm on"})
                    } else {
                        let mut a = alarm(r["alarm"].as_str()?, "device_restarted", "cleared");
                        a["acknowledged_by"] = json!("wp-p2cc2");
                        a["cleared_at"] = json!("2026-11-15T10:00:00Z");
                        json!({"v": 1, "alarm": a})
                    }
                }
                "alarm.rules.get" | "alarm.rules.set" => {
                    if r["rules"]["cpu"]["above_pct"] == 101 {
                        json!({"code": "bad_request", "msg": "rules.cpu.above_pct must be a whole number 1 to 100"})
                    } else {
                        json!({"v": 1, "rules": {
                            "device_offline": {"on": true, "after_s": 30},
                            "clip": {"on": true, "per_minute": 10, "mute_on_storm": false},
                            "cpu": {"on": true, "above_pct": 90, "for_s": 10},
                            "brand_new": {"on": true}}})
                    }
                }
                "schedule.list" => json!({"v": 1, "rules": [{"id": "r1", "name": "Doors open",
                    "enabled": true, "when": {"days": ["mon"], "time": "18:30"},
                    "actions": [{"t": "preset.load", "slot": 2}], "targets": [SERIAL],
                    "next_run": null, "last": null}],
                    "zone": "Europe/Stockholm", "local_time": "2026-11-15T10:00", "clock_unset": false}),
                "schedule.set" => {
                    let mut rule = r["rule"].clone();
                    if rule.get("id").is_none() {
                        rule["id"] = json!("r2");
                    }
                    rule["next_run"] = json!("2026-11-16T17:30:00Z");
                    rule["last"] = Value::Null;
                    json!({"v": 1, "rule": rule})
                }
                "schedule.delete" => json!({"v": 1, "rule": r["rule"]}),
                "schedule.run_now" => json!({"v": 1, "rule": r["rule"], "name": "Doors open",
                    "at": "2026-11-15T10:00:00Z", "by": "wp-p2cc2", "results": [
                        {"serial": SERIAL, "action": "0:preset.load", "ok": true},
                        {"serial": "OTHER", "action": "0:preset.load", "ok": false,
                         "code": "offline", "msg": "not attached"}]}),
                "outbound.get" => json!({"v": 1, "targets": [{"id": "t1", "name": "Ops",
                    "kind": "push", "url": "https://ntfy.example/dspi", "enabled": true,
                    "kinds": ["alarm.raised"], "secret": "not set", "queued": 0, "dropped": 0,
                    "last": null, "encrypted": true, "plaintext": false, "trust": "webpki"}]}),
                "outbound.set" => {
                    let mut targets = r["targets"].as_array()?.clone();
                    let mut secrets = serde_json::Map::new();
                    for (i, t) in targets.iter_mut().enumerate() {
                        if t.get("id").is_none() {
                            t["id"] = json!(format!("t{}", i + 1));
                        }
                        if t["secret"] == "new" {
                            secrets.insert(t["id"].as_str()?.to_owned(), json!("ab".repeat(32)));
                            t["secret"] = json!("set");
                        }
                    }
                    json!({"v": 1, "targets": targets, "secrets": secrets})
                }
                "outbound.test" => {
                    json!({"v": 1, "delivered": false, "status": 404, "msg": "HTTP 404"})
                }
                "mqtt.get" | "mqtt.set" => {
                    let mut s = json!({"enabled": false, "broker": "", "port": 1883,
                        "client_id": "dspiabc", "username": null, "password": "not set",
                        "base_topic": "dspi/abc", "ha": {"enabled": false, "key": null},
                        "tls": {"mode": "off", "client_key_pem": "not set"}});
                    if t == "mqtt.set" {
                        let p = &r["settings"];
                        s["broker"] = p["broker"].clone();
                        s["enabled"] = p["enabled"].clone();
                        s["username"] = p["username"].clone();
                        if p["password"].is_string() {
                            s["password"] = json!("set");
                        }
                    }
                    json!({"v": 1, "settings": s})
                }
                "mqtt.status" => {
                    json!({"v": 1, "state": "failed", "msg": "the broker refused the password",
                    "since": "2026-11-15T10:00:00Z", "queued": 3, "dropped": 0, "encrypted": false})
                }
                "auth.list" => json!({"clients": [
                    {"cid": 1, "name": "Phone", "role": "control", "created": "2026-11-01T10:00:00Z",
                     "online": true, "kind": "console", "plaintext_ok": false},
                    {"cid": 5, "name": "Home Assistant", "role": "control", "created": "2026-11-02T10:00:00Z",
                     "online": false, "kind": "api", "plaintext_ok": false}]}),
                "auth.mint" => {
                    if r["role"] == "admin" {
                        json!({"code": "denied", "msg": "a key's role is never above the minting session's"})
                    } else {
                        json!({"v": 1, "cid": 6, "name": r["name"], "role": r["role"], "kind": "api",
                               "plaintext_ok": r["plaintext_ok"], "key": "K".repeat(43)})
                    }
                }
                "auth.revoke" => json!({}),
                _ => return None,
            })
        })
    }

    #[test]
    fn wp_p2cc2_without_the_caps_every_call_is_unsupported_and_sends_nothing() {
        let hub = hub(false);
        hub.set_json_responder(Some(answers()));
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.on_hub, "{caps:?}");
        assert!(!caps.alarms && !caps.schedule && !caps.outbound && !caps.mqtt);
        assert!(!caps.api_keys && !caps.rest);
        let u = DspiError::Unsupported;
        assert_eq!(
            core.hub_alarms(String::new(), String::new()).unwrap_err(),
            u
        );
        assert_eq!(core.hub_alarm_ack("device:1".into()).unwrap_err(), u);
        assert_eq!(core.hub_alarm_rules_get().unwrap_err(), u);
        assert_eq!(core.hub_alarm_rules_set(Vec::new()).unwrap_err(), u);
        assert_eq!(core.hub_schedules().unwrap_err(), u);
        assert_eq!(
            core.hub_schedule_set(HubScheduleRule::default())
                .unwrap_err(),
            u
        );
        assert_eq!(core.hub_schedule_delete("r1".into()).unwrap_err(), u);
        assert_eq!(core.hub_schedule_run_now("r1".into()).unwrap_err(), u);
        assert_eq!(core.hub_outbound_get().unwrap_err(), u);
        assert_eq!(core.hub_outbound_set(Vec::new()).unwrap_err(), u);
        assert_eq!(core.hub_outbound_test("t1".into()).unwrap_err(), u);
        assert_eq!(core.hub_mqtt_get().unwrap_err(), u);
        assert_eq!(
            core.hub_mqtt_set(HubMqttSettings::default()).unwrap_err(),
            u
        );
        assert_eq!(core.hub_mqtt_status().unwrap_err(), u);
        assert_eq!(core.hub_api_keys().unwrap_err(), u);
        assert_eq!(
            core.hub_api_key_mint("k".into(), "viewer".into(), false)
                .unwrap_err(),
            u
        );
        assert_eq!(core.hub_api_key_revoke(5).unwrap_err(), u);
        assert!(sent(&hub).is_empty(), "{:?}", sent(&hub));
        core.link_close();
    }

    #[test]
    fn wp_p2cc2_each_call_with_the_cap_through_the_fake_hub() {
        let hub = hub(true);
        hub.set_json_responder(Some(answers()));
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.alarms && caps.schedule && caps.outbound && caps.mqtt && caps.api_keys);
        assert!(!caps.rest && !caps.inventory);

        // Alarms.
        let alarms = core.hub_alarms(SERIAL.into(), "active".into()).unwrap();
        assert_eq!(alarms.len(), 2);
        assert_eq!(
            (alarms[0].reason.as_str(), alarms[0].notice_key.as_str()),
            ("glitch", "alarm.device_restarted")
        );
        assert_eq!(
            (
                alarms[1].source.as_str(),
                alarms[1].acknowledged_by.as_str()
            ),
            ("alink", "Phone")
        );
        assert_eq!(alarms[1].serial, "");
        let ask = last(&hub, "alarm.list");
        assert_eq!(
            (ask["serial"].clone(), ask["state"].clone()),
            (json!(SERIAL), json!("active"))
        );
        core.hub_alarms(String::new(), String::new()).unwrap();
        let ask = last(&hub, "alarm.list");
        assert!(
            ask.get("serial").is_none() && ask.get("state").is_none(),
            "{ask}"
        );
        let acked = core.hub_alarm_ack("device:1".into()).unwrap();
        assert_eq!(
            (acked.state.as_str(), acked.acknowledged_by.as_str()),
            ("cleared", "wp-p2cc2")
        );
        assert_eq!(last(&hub, "alarm.ack")["alarm"], "device:1");
        let e = core.hub_alarm_ack("device:9".into()).unwrap_err();
        assert_eq!(e.notice_key(), "dlink.not_found", "{e:?}");
        let rules = core.hub_alarm_rules_get().unwrap();
        let codes: Vec<&str> = rules.iter().map(|r| r.code.as_str()).collect();
        assert_eq!(codes, ["device_offline", "clip", "cpu", "brand_new"]);
        assert_eq!(rules[2].above_pct, Some(90));
        let mut cpu = rules[2].clone();
        cpu.above_pct = Some(101);
        let e = core.hub_alarm_rules_set(vec![cpu.clone()]).unwrap_err();
        assert!(
            matches!(&e, DspiError::InvalidArgument { msg } if msg.contains("above_pct")),
            "{e:?}"
        );
        cpu.above_pct = Some(80);
        core.hub_alarm_rules_set(vec![cpu]).unwrap();
        assert_eq!(
            last(&hub, "alarm.rules.set")["rules"],
            json!({"cpu": {"on": true, "above_pct": 80, "for_s": 10}})
        );

        // Schedules.
        let s = core.hub_schedules().unwrap();
        assert_eq!(
            (s.zone.as_str(), s.clock_unset),
            ("Europe/Stockholm", false)
        );
        assert_eq!(s.rules[0].actions[0].slot, Some(2));
        assert_eq!(s.rules[0].next_run, "");
        let new = HubScheduleRule {
            name: "Close".into(),
            enabled: true,
            once: "2026-12-24T17:00".into(),
            actions: vec![HubScheduleAction {
                t: "param.set".into(),
                path: "vol.mute".into(),
                flag: Some(true),
                ..HubScheduleAction::default()
            }],
            targets: vec![SERIAL.into()],
            ..HubScheduleRule::default()
        };
        let made = core.hub_schedule_set(new).unwrap();
        assert_eq!(
            (made.id.as_str(), made.next_run.as_str()),
            ("r2", "2026-11-16T17:30:00Z")
        );
        assert_eq!(made.actions[0].flag, Some(true));
        let ask = last(&hub, "schedule.set");
        assert!(ask["rule"].get("id").is_none(), "{ask}");
        assert!(ask["rule"].get("next_run").is_none(), "{ask}");
        core.hub_schedule_delete("r2".into()).unwrap();
        assert_eq!(last(&hub, "schedule.delete")["rule"], "r2");
        let run = core.hub_schedule_run_now("r1".into()).unwrap();
        assert_eq!((run.rule.as_str(), run.by.as_str()), ("r1", "wp-p2cc2"));
        assert_eq!(run.results.len(), 2);
        assert!(!run.results[1].ok);
        assert_eq!(run.results[1].code, "offline");

        // Notifications out: a new secret comes once.
        let targets = core.hub_outbound_get().unwrap();
        assert_eq!(
            (targets[0].kind.as_str(), targets[0].trust.as_str()),
            ("push", "webpki")
        );
        let mut list = targets.clone();
        list.push(HubOutboundTarget {
            kind: "webhook".into(),
            url: "https://ops.example/hook".into(),
            enabled: true,
            kinds: vec!["alarm.raised".into()],
            secret: "new".into(),
            ..HubOutboundTarget::default()
        });
        let set = core.hub_outbound_set(list).unwrap();
        assert_eq!(set.targets.len(), 2);
        assert_eq!(set.secrets.len(), 1);
        assert_eq!(set.secrets[0].target, "t2");
        assert_eq!(set.secrets[0].secret.len(), 64);
        assert_eq!(set.targets[1].secret, "set");
        let ask = last(&hub, "outbound.set");
        assert_eq!(
            ask["targets"][0]["secret"], "not set",
            "sent back as it was: {ask}"
        );
        assert!(ask["targets"][0].get("queued").is_none(), "{ask}");
        let test = core.hub_outbound_test("t1".into()).unwrap();
        assert_eq!((test.delivered, test.status), (false, Some(404)));

        // MQTT: the password goes out once typed, and comes back as `set`.
        let mut m = core.hub_mqtt_get().unwrap();
        assert_eq!((m.password.as_str(), m.port), ("not set", Some(1883)));
        m.enabled = true;
        m.broker = "10.0.0.2".into();
        m.username = "hub".into();
        m.password = "broker-pass".into();
        let m = core.hub_mqtt_set(m).unwrap();
        assert_eq!(
            (m.broker.as_str(), m.password.as_str()),
            ("10.0.0.2", "set")
        );
        let ask = last(&hub, "mqtt.set");
        assert_eq!(ask["settings"]["password"], "broker-pass");
        assert_eq!(ask["settings"]["tls"]["mode"], "off");
        let st = core.hub_mqtt_status().unwrap();
        assert_eq!((st.state.as_str(), st.queued), ("failed", 3));
        assert!(st.msg.contains("password"));

        // API keys: only the keys listed; the key comes once; a console's
        // pairing is never revoked by this call.
        let keys = core.hub_api_keys().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!((keys[0].cid, keys[0].name.as_str()), (5, "Home Assistant"));
        let minted = core
            .hub_api_key_mint("Script".into(), "viewer".into(), false)
            .unwrap();
        assert_eq!((minted.cid, minted.key.len()), (6, 43));
        let ask = last(&hub, "auth.mint");
        assert_eq!(
            (ask["role"].clone(), ask["plaintext_ok"].clone()),
            (json!("viewer"), json!(false))
        );
        assert_eq!(
            core.hub_api_key_mint("Root".into(), "admin".into(), false)
                .unwrap_err(),
            DspiError::Denied
        );
        core.hub_api_key_revoke(5).unwrap();
        assert_eq!(last(&hub, "auth.revoke")["cid"], 5);
        let before = sent(&hub)
            .iter()
            .filter(|r| r["t"] == "auth.revoke")
            .count();
        assert!(matches!(
            core.hub_api_key_revoke(1),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert_eq!(
            sent(&hub)
                .iter()
                .filter(|r| r["t"] == "auth.revoke")
                .count(),
            before
        );
        core.link_close();
    }

    #[test]
    fn wp_p2cc2_read_only_mode_refuses_every_write_family_and_sends_nothing() {
        let hub = hub(true);
        hub.set_json_responder(Some(answers()));
        let core = bound(&hub);
        core.set_client_read_only(true).unwrap();
        let writes = || {
            sent(&hub)
                .into_iter()
                .filter(|r| {
                    !crate::hub_server::HUB_SERVER_READS.contains(&r["t"].as_str().unwrap_or(""))
                })
                .count()
        };
        let r = DspiError::ReadOnly;
        assert_eq!(core.hub_alarm_ack("device:1".into()).unwrap_err(), r);
        assert_eq!(
            core.hub_alarm_rules_set(vec![HubAlarmRule {
                code: "cpu".into(),
                on: false,
                ..HubAlarmRule::default()
            }])
            .unwrap_err(),
            r
        );
        let rule = HubScheduleRule {
            name: "x".into(),
            days: vec!["mon".into()],
            time: "10:00".into(),
            ..HubScheduleRule::default()
        };
        assert_eq!(core.hub_schedule_set(rule).unwrap_err(), r);
        assert_eq!(core.hub_schedule_delete("r1".into()).unwrap_err(), r);
        assert_eq!(core.hub_schedule_run_now("r1".into()).unwrap_err(), r);
        assert_eq!(core.hub_outbound_set(Vec::new()).unwrap_err(), r);
        assert_eq!(core.hub_outbound_test("t1".into()).unwrap_err(), r);
        assert_eq!(
            core.hub_mqtt_set(HubMqttSettings::default()).unwrap_err(),
            r
        );
        assert_eq!(
            core.hub_api_key_mint("k".into(), "viewer".into(), false)
                .unwrap_err(),
            r
        );
        assert_eq!(core.hub_api_key_revoke(5).unwrap_err(), r);
        // Every write message of these families, however it is sent.
        for t in [
            "alarm.ack",
            "alarm.rules.set",
            "schedule.set",
            "schedule.delete",
            "schedule.run_now",
            "outbound.set",
            "outbound.test",
            "mqtt.set",
            "auth.mint",
            "auth.revoke",
            "auth.set_role",
        ] {
            assert_eq!(
                core.link_hub_request(t, serde_json::Map::new())
                    .unwrap_err(),
                r,
                "{t}"
            );
        }
        assert_eq!(writes(), 0, "{:?}", sent(&hub));
        // The reads go on.
        assert_eq!(
            core.hub_alarms(String::new(), String::new()).unwrap().len(),
            2
        );
        core.hub_alarm_rules_get().unwrap();
        core.hub_schedules().unwrap();
        core.hub_outbound_get().unwrap();
        core.hub_mqtt_get().unwrap();
        core.hub_mqtt_status().unwrap();
        assert_eq!(core.hub_api_keys().unwrap().len(), 1);
        assert_eq!(writes(), 0);
        core.link_close();
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<HubServerEvent>>);

    impl HubServerListener for Heard {
        fn on_hub_server_event(&self, event: HubServerEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn wp_p2cc2_alarm_changed_and_schedule_ran_reach_the_listener() {
        let hub = hub(true);
        let core = bound(&hub);
        let heard = Arc::new(Heard::default());
        core.set_hub_server_listener(Some(heard.clone()));
        hub.send_event(json!({"t": "alarm.changed", "v": 1, "hub": "h",
            "alarm": alarm("device:2", "device_restarted", "raised")}));
        hub.send_event(
            json!({"t": "schedule.ran", "v": 1, "hub": "h", "rule": "r1",
            "name": "Doors open", "at": "2026-11-15T18:30:00Z", "by": null,
            "results": [{"serial": SERIAL, "action": "0:preset.load", "ok": true}]}),
        );
        wait_until("two events", || heard.0.lock().unwrap().len() >= 2);
        let got = heard.0.lock().unwrap().clone();
        assert_eq!(got[0].kind, "alarm.changed");
        assert_eq!(got[0].serial, SERIAL);
        let a = got[0].alarm.clone().unwrap();
        assert_eq!((a.id.as_str(), a.reason.as_str()), ("device:2", "glitch"));
        assert!(got[0].run.is_none());
        assert_eq!(got[1].kind, "schedule.ran");
        let run = got[1].run.clone().unwrap();
        assert_eq!((run.rule.as_str(), run.by.as_str()), ("r1", ""));
        assert!(run.results[0].ok);
        assert!(got[1].alarm.is_none());
        let queued = core.hub_server_events();
        assert_eq!(queued.len(), 2);
        // C and the web see the same object.
        let v = serde_json::to_value(&queued[0]).unwrap();
        assert_eq!(v["alarm"]["noticeKey"], "alarm.device_restarted", "{v}");
        core.set_hub_server_listener(None);
        core.link_close();
    }
}
