//! WP P2MQ: the broker setting, its file and what is published, without sockets.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use dspi_session::events::{EventKind, EventRecord};

fn scratch(name: &str) -> PathBuf {
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let d = std::env::temp_dir()
        .join("wp-p2mq-unit")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

const HUB: &str = "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b";

#[test]
fn wp_p2mq_defaults_are_off_with_the_documented_topic_and_port() {
    let s = MqttSettings::default();
    assert!(!s.enabled);
    assert!(!s.active());
    let v = s.view(HUB);
    assert_eq!(v["port"], 1883);
    assert_eq!(v["base_topic"], "dspi/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b");
    assert_eq!(v["password"], "not set");
    assert_eq!(v["ha"], json!({"enabled": false, "key": null}));
    // -- WP MQWIRE --
    assert_eq!(v["tls"]["mode"], "off", "{v}");
    assert_eq!(v["tls"]["encrypted"], false, "{v}");
    let id = s.client(HUB);
    assert!(id.len() <= 23 && id.starts_with("dspi"), "{id}");
    assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn wp_p2mq_set_is_in_part_and_all_or_nothing() {
    let s0 = MqttSettings::default();
    let s1 = apply_request(
        &s0,
        &json!({"enabled": true, "broker": "10.0.0.5", "username": "hub", "password": "pw-77"}),
    )
    .unwrap();
    assert!(s1.active());
    assert_eq!(s1.port, None, "left out, kept");
    assert_eq!(s1.port_in_use(), 1883);
    assert_eq!(s1.password.as_deref(), Some("pw-77"));
    // `mqtt.get`'s answer sent back keeps the password.
    let back = apply_request(&s1, &s1.view(HUB)).unwrap();
    assert_eq!(back.password.as_deref(), Some("pw-77"));
    // `null` clears it, as does "not set".
    let cleared = apply_request(&s1, &json!({"password": null})).unwrap();
    assert_eq!(cleared.password, None);
    let cleared = apply_request(&s1, &json!({"password": "not set"})).unwrap();
    assert_eq!(cleared.password, None);
    // One bad field: nothing changes.
    for bad in [
        json!({"port": 0}),
        json!({"port": 70000}),
        json!({"broker": "a b"}),
        json!({"broker": 5}),
        json!({"base_topic": "a/#"}),
        json!({"base_topic": "a//b"}),
        json!({"base_topic": "/a"}),
        json!({"base_topic": "$SYS/x"}),
        json!({"client_id": ""}),
        json!({"client_id": "a b"}),
        json!({"username": "x\ny"}),
        json!({"password": "x".repeat(300)}),
        json!({"enabled": "yes"}),
        json!({"ha": {"key": -1}}),
        json!({"ha": true}),
        json!({"enabled": true, "broker": ""}),
        json!({"username": null}),
        json!("not an object"),
    ] {
        let err = apply_request(&s1, &bad);
        assert!(err.is_err(), "{bad} was taken");
        assert!(
            !err.unwrap_err().contains("pw-77"),
            "no password in a reason"
        );
    }
    // -- WP MQWIRE -- `tls` in part, through `MqttTls::update`; no port: 8883 with TLS.
    let t = apply_request(&s1, &json!({"tls": {"mode": "on"}})).unwrap();
    assert!(t.tls.encrypted());
    assert_eq!(t.port_in_use(), 8883);
    assert!(!t.same_connection(&s1));
    let bad = apply_request(&s1, &json!({"tls": {"mode": "maybe"}})).unwrap_err();
    assert!(bad.starts_with("tls:"), "{bad}");
    assert!(
        apply_request(&s1, &json!({"tls": {"ca_pem": "x"}})).is_err(),
        "off with a CA"
    );
    // `ha` alone does not reconnect; a port does.
    let ha = apply_request(&s1, &json!({"ha": {"enabled": true, "key": 3}})).unwrap();
    assert_eq!(
        ha.ha,
        HaSetting {
            enabled: true,
            key: Some(3)
        }
    );
    assert!(ha.same_connection(&s1));
    let port = apply_request(&s1, &json!({"port": 1884})).unwrap();
    assert!(!port.same_connection(&s1));
}

#[test]
fn wp_p2mq_debug_never_shows_the_password() {
    let s = apply_request(
        &MqttSettings::default(),
        &json!({"broker": "b", "username": "u", "password": "sekrit-p2mq"}),
    )
    .unwrap();
    let d = format!("{s:?}");
    assert!(!d.contains("sekrit-p2mq"), "{d}");
    assert!(d.contains("\"set\""), "{d}");
    let c = format!("{:?}", s.client_config(HUB));
    assert!(!c.contains("sekrit-p2mq"), "{c}");
    assert!(!s.view(HUB).to_string().contains("sekrit-p2mq"));
}

#[test]
fn wp_p2mq_the_file_is_0600_keeps_tls_and_reads_back() {
    let d = scratch("file");
    let mut s = apply_request(
        &MqttSettings::default(),
        &json!({"enabled": true, "broker": "broker.lan", "port": 1884, "username": "u",
                "password": "pw", "base_topic": "venue/a", "ha": {"enabled": true}}),
    )
    .unwrap();
    s.tls = MqttTls::update(
        &json!({"mode": "off", "allow_plaintext_password": true}),
        &s.tls,
    )
    .unwrap();
    save(&d, &s).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(d.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let (back, notice) = load(&d, 1);
    assert_eq!(back, s);
    assert_eq!(notice, None);
    assert!(back.tls.allow_plaintext_password, "kept on rewrite");
    // A rewrite through `mqtt.set` keeps `tls` too.
    let next = apply_request(&back, &json!({"port": 1885})).unwrap();
    save(&d, &next).unwrap();
    assert!(load(&d, 2).0.tls.allow_plaintext_password);
}

#[test]
fn wp_p2mq_a_bad_file_is_set_aside_and_its_good_fields_kept() {
    let d = scratch("bad");
    fs::write(
        d.join(FILE_NAME),
        r#"{"v":1,"enabled":true,"broker":"b.lan","port":99999,"username":"u","password":"pw"}"#,
    )
    .unwrap();
    let (s, notice) = load(&d, 42);
    assert_eq!(s.broker, "b.lan");
    assert_eq!(s.port, None, "the bad field reads as its default");
    assert!(notice.is_some(), "no tls, a remote broker: migrated");
    assert_eq!(s.password.as_deref(), Some("pw"));
    assert!(d.join("mqtt.json.bad-42").exists());
    assert_eq!(load(&d, 43), (s, None), "the good fields written back");

    // Not JSON at all: defaults, set aside, never a panic.
    fs::write(d.join(FILE_NAME), b"\xff{{").unwrap();
    assert_eq!(load(&d, 44).0, MqttSettings::default());
    assert!(d.join("mqtt.json.bad-44").exists());

    // Too large: defaults.
    fs::write(d.join(FILE_NAME), vec![b' '; MAX_FILE as usize + 1]).unwrap();
    assert_eq!(load(&d, 45).0, MqttSettings::default());

    // A password without a user name cannot be sent: off, and the password dropped.
    fs::write(
        d.join(FILE_NAME),
        r#"{"enabled":true,"broker":"b","password":"pw"}"#,
    )
    .unwrap();
    let (s, _) = load(&d, 46);
    assert!(!s.enabled);
    assert_eq!(s.password, None);
}

#[test]
fn wp_p2mq_status_follows_the_client() {
    let mut st = StatusView::new(10);
    assert_eq!(st.state, LinkState::Off);
    st.set(LinkState::Connecting, None, 11);
    st.on_event(
        &MqttEvent::Connected {
            session_present: false,
        },
        12,
    );
    assert_eq!((st.state, st.since), (LinkState::Connected, 12));
    st.on_event(
        &MqttEvent::Disconnected {
            reason: "the broker refused the connection: bad user name or password".into(),
        },
        13,
    );
    assert_eq!(st.state, LinkState::Failed);
    assert!(
        st.msg
            .as_deref()
            .unwrap()
            .contains("bad user name or password")
    );
    st.on_event(
        &MqttEvent::Disconnected {
            reason: "the broker refused the connection: not authorised".into(),
        },
        14,
    );
    assert_eq!(st.since, 13, "the same state keeps its since");
    st.on_event(
        &MqttEvent::Disconnected {
            reason: "connection reset".into(),
        },
        15,
    );
    assert_eq!(st.state, LinkState::Connecting);
    let j = Value::Object(st.to_json());
    assert_eq!(j["state"], "connecting");
    assert_eq!(j["encrypted"], false);
    assert_eq!(j["since"], "1970-01-01T00:00:15Z");
}

#[test]
fn wp_p2mq_the_handler_queue_is_bounded() {
    let q = EventQueue::default();
    for n in 0..200 {
        q.push(MqttEvent::Disconnected {
            reason: format!("r{n}"),
        });
    }
    q.push(MqttEvent::Message {
        topic: "x".into(),
        payload: vec![],
        retain: false,
    });
    let got = q.drain();
    assert_eq!(got.len(), EVENT_QUEUE_CAP);
    assert_eq!(
        got.last(),
        Some(&MqttEvent::Disconnected {
            reason: "r199".into()
        }),
        "the newest stay; a message is not kept"
    );
}

fn alarm(serial: Option<&str>, state: AlarmState) -> BusMessage {
    BusMessage::Alarm {
        id: "device:3".into(),
        source: "device".into(),
        code: "temperature".into(),
        subject: serial.unwrap_or("alink").into(),
        serial: serial.map(str::to_owned),
        severity: "critical".into(),
        state,
        message: "72 °C".into(),
        at: 100,
        raised_at: 90,
        cleared_at: None,
        acknowledged_by: None,
        count: 1,
    }
}

#[test]
fn wp_p2mq_what_is_published_where() {
    let (topic, payload, qos) = publication(
        &alarm(Some("E46058388B1A2E2C"), AlarmState::Raised),
        "dspi/h",
        "h",
    )
    .unwrap();
    assert_eq!(topic, "dspi/h/E46058388B1A2E2C/event");
    assert_eq!(qos, QoS::AtLeastOnce);
    let v: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(v["kind"], "alarm");
    assert_eq!(v["v"], 1);
    assert_eq!(v["hub"], "h");
    assert_eq!(v["code"], "temperature");
    assert_eq!(v["state"], "raised");
    assert_eq!(v["serial"], "E46058388B1A2E2C");
    assert_eq!(v["raised_at"], "1970-01-01T00:01:30Z");

    let (topic, ..) = publication(&alarm(None, AlarmState::Cleared), "dspi/h", "h").unwrap();
    assert_eq!(topic, "dspi/h/hub/event", "an alarm about no one device");
    assert!(publication(&alarm(Some("a/b"), AlarmState::Raised), "b", "h").is_none());
    assert!(publication(&alarm(Some("#"), AlarmState::Raised), "b", "h").is_none());

    let record = EventRecord {
        kind: EventKind::default(),
        text: "Preset 2 loaded".into(),
        serial: "S1".into(),
        ..serde_json::from_value(json!({})).unwrap()
    };
    let (topic, payload, qos) = publication(
        &BusMessage::Event {
            serial: "S1".into(),
            record,
        },
        "dspi/h",
        "h",
    )
    .unwrap();
    assert_eq!(topic, "dspi/h/S1/event");
    assert_eq!(qos, QoS::AtMostOnce);
    let v: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(v["kind"], "event");
    assert_eq!(v["record"]["text"], "Preset 2 loaded");

    assert!(
        publication(
            &BusMessage::DeviceOnline {
                serial: "S1".into(),
                handle: 1
            },
            "b",
            "h"
        )
        .is_none()
    );
}

#[test]
fn wp_p2mq_the_will_is_offline_retained_on_the_status_topic() {
    let s = apply_request(
        &MqttSettings::default(),
        &json!({"broker": "b", "base_topic": "v/a"}),
    )
    .unwrap();
    let c = s.client_config(HUB);
    let will = c.will.unwrap();
    assert_eq!(will.topic, "v/a/status");
    assert_eq!(will.payload, OFFLINE);
    assert!(will.retain);
    assert_eq!(will.qos, QoS::AtLeastOnce);
}

// -- WP MQWIRE --
#[test]
fn wp_mqwire_a_file_from_before_tls_is_migrated_once() {
    let d = scratch("migrate");
    // As P2MQ wrote it: `tls` null, a broker elsewhere with a password.
    fs::write(
        d.join(FILE_NAME),
        r#"{"v":1,"enabled":true,"broker":"10.0.0.5","port":1883,"username":"u","password":"pw","tls":null}"#,
    )
    .unwrap();
    let (s, notice) = load(&d, 1);
    assert_eq!(notice, Some(super::super::client::MQTT_MIGRATED_NOTICE));
    assert!(s.tls.allow_plaintext_password);
    assert!(s.enabled);
    assert!(
        !d.join("mqtt.json.bad-1").exists(),
        "migrated, not set aside"
    );
    let (again, notice) = load(&d, 2);
    assert_eq!(notice, None, "the notice comes once");
    assert_eq!(again, s);

    // On loopback, or with no password: nothing to migrate.
    for text in [
        r#"{"enabled":true,"broker":"127.0.0.1","username":"u","password":"pw"}"#,
        r#"{"enabled":true,"broker":"10.0.0.5"}"#,
    ] {
        fs::write(d.join(FILE_NAME), text).unwrap();
        let (s, notice) = load(&d, 3);
        assert_eq!(notice, None, "{text}");
        assert!(!s.tls.allow_plaintext_password, "{text}");
    }
}
