//! WP P2HA: Home Assistant's pure parts: topics, `set` parsing of hostile input,
//! discovery payloads, state from the bus, removal, the bounded queues.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use proptest::prelude::*;
use serde_json::{Value, json};

use super::*;
use crate::mqtt::service::HaSetting;

const BASE: &str = "dspi/hub1";
const HUB: &str = "hub1";
const SERIAL: &str = "E6614C311B7A8C22";

fn param_changed(offset: usize, bytes: &[u8]) -> Vec<u8> {
    // version 2, PARAM_CHANGED, flags, seq, offset, size, source, 3 reserved, value.
    let mut p = vec![2u8, g::notify::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
    p.extend_from_slice(&u16::try_from(offset).unwrap().to_le_bytes());
    p.extend_from_slice(&u16::try_from(bytes.len()).unwrap().to_le_bytes());
    p.extend_from_slice(&[g::notify::PARAM_SRC_HOST_SET as u8, 0, 0, 0]);
    p.extend_from_slice(bytes);
    p
}

fn notification(packet: Vec<u8>) -> BusMessage {
    BusMessage::Notification {
        serial: SERIAL.into(),
        session: 300,
        by: Some("Home Assistant".into()),
        packet: Arc::from(packet),
    }
}

fn online() -> BusMessage {
    BusMessage::DeviceOnline {
        serial: SERIAL.into(),
        handle: 1,
    }
}

fn topics(out: &mut Outbox) -> BTreeMap<String, String> {
    out.drain()
        .into_iter()
        .map(|(t, p)| (t, String::from_utf8(p).unwrap()))
        .collect()
}

fn state_with_device(out: &mut Outbox) -> HaState {
    let mut st = HaState::new(BASE, HUB);
    assert_eq!(st.on_bus(out, &online()), vec![Job::Refresh(SERIAL.into())]);
    st.on_feedback(
        out,
        Feedback::Device {
            serial: SERIAL.into(),
            name: "Front fills".into(),
            fw: "1.1.7".into(),
            platform: "RP2350".into(),
            model: None,
            online: true,
        },
    );
    st.on_feedback(
        out,
        Feedback::Values {
            serial: SERIAL.into(),
            volume: Some(-20.0),
            mute: Some(false),
            presets: Some(preset_labels(&[
                (0, "House".into(), true),
                (1, String::new(), true),
                (2, "Unused".into(), false),
            ])),
            active: Some(Some(0)),
        },
    );
    st
}

#[test]
fn wp_p2ha_topics_and_ids() {
    assert_eq!(
        config_topic("hub/1+", SERIAL, Entity::MasterVolume),
        format!("homeassistant/number/hub1_{SERIAL}_master_volume/config")
    );
    assert_eq!(
        state_topic(BASE, SERIAL, Entity::Mute),
        format!("{BASE}/{SERIAL}/mute/state")
    );
    assert_eq!(
        set_topic(BASE, SERIAL, Entity::Preset),
        format!("{BASE}/{SERIAL}/preset/set")
    );
    assert_eq!(set_filter(BASE), "dspi/hub1/+/+/set");
    assert_eq!(
        availability_topic(BASE, SERIAL),
        format!("{BASE}/{SERIAL}/availability")
    );
    for e in Entity::ALL {
        assert_eq!(Entity::from_word(e.word()), Some(e));
    }
    assert_eq!(serial_level("a/b"), None);
    assert_eq!(serial_level("a+"), None);
    assert_eq!(serial_level(""), None);
    assert_eq!(serial_level(&"x".repeat(65)), None);
    assert_eq!(volume_range(), (-128.0, 0.0), "the registry's range");
}

#[test]
fn wp_p2ha_set_payloads_are_typed_and_bounded() {
    let t = |e: Entity| set_topic(BASE, SERIAL, e);
    assert_eq!(
        parse_set(BASE, &t(Entity::MasterVolume), b" -25.5 ").unwrap(),
        (SERIAL.into(), Entity::MasterVolume, Command::Volume(-25.5))
    );
    assert_eq!(
        parse_set(BASE, &t(Entity::Mute), b"ON").unwrap().2,
        Command::Mute(true)
    );
    assert_eq!(
        parse_set(BASE, &t(Entity::Mute), b"off").unwrap().2,
        Command::Mute(false)
    );
    assert_eq!(
        parse_set(BASE, &t(Entity::Preset), "Bühne".as_bytes())
            .unwrap()
            .2,
        Command::Preset("Bühne".into())
    );
    assert_eq!(
        parse_set(BASE, &t(Entity::Identify), b"PRESS").unwrap().2,
        Command::Identify
    );
    // Out of range, not finite, not a number.
    for bad in [&b"1"[..], b"-129", b"NaN", b"inf", b"-1e400", b"loud", b""] {
        assert!(
            matches!(
                parse_set(BASE, &t(Entity::MasterVolume), bad),
                Err(InboundError::BadValue(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        parse_set(BASE, &t(Entity::Mute), b"1"),
        Err(InboundError::BadValue(_))
    ));
    assert!(matches!(
        parse_set(BASE, &t(Entity::Preset), b"a\x07b"),
        Err(InboundError::BadValue(_))
    ));
    assert_eq!(
        parse_set(BASE, &t(Entity::Mute), &[b'O'; MAX_PAYLOAD + 1]),
        Err(InboundError::TooLong)
    );
    assert_eq!(
        parse_set(BASE, &t(Entity::Mute), &[0xFF, 0xFE]),
        Err(InboundError::NotText)
    );
    // Not a settable entity, not ours, a wrong shape.
    for topic in [
        state_topic(BASE, SERIAL, Entity::Temperature).replace("/state", "/set"),
        format!("{BASE}/{SERIAL}/mute/state"),
        format!("other/{SERIAL}/mute/set"),
        format!("{BASE}/{SERIAL}/mute/set/x"),
        format!("{BASE}/{SERIAL}/set"),
        format!("{BASE}//mute/set"),
        format!("{BASE}/{SERIAL}/nothing/set"),
    ] {
        assert_eq!(
            parse_set(BASE, &topic, b"ON"),
            Err(InboundError::NotOurs),
            "{topic}"
        );
    }
}

proptest! {
    #[test]
    fn wp_p2ha_any_topic_and_payload_is_an_answer_never_a_panic(
        topic in "\\PC{0,80}",
        tail in prop::sample::select(vec!["master_volume/set", "mute/set", "preset/set",
            "identify/set", "clip/set", "x"]),
        payload in prop::collection::vec(any::<u8>(), 0..100),
    ) {
        let _ = parse_set(BASE, &topic, &payload);
        let full = format!("{BASE}/{topic}/{tail}");
        let parsed = parse_set(BASE, &full, &payload);
        if let Ok((serial, entity, command)) = parsed {
            prop_assert!(payload.len() <= MAX_PAYLOAD);
            prop_assert!(serial_level(&serial).is_some());
            prop_assert!(entity.settable());
            if let Command::Volume(v) = command {
                prop_assert!(v.is_finite() && (-128.0..=0.0).contains(&v));
            }
        }
    }
}

#[test]
fn wp_p2ha_discovery_has_the_device_block_and_every_entity() {
    let mut out = Outbox::default();
    let _st = state_with_device(&mut out);
    let got = topics(&mut out);
    for e in Entity::ALL {
        let topic = config_topic(HUB, SERIAL, e);
        let c: Value = serde_json::from_str(&got[&topic]).unwrap_or_else(|_| panic!("{topic}"));
        assert_eq!(c["unique_id"], format!("hub1_{SERIAL}_{}", e.word()));
        assert_eq!(c["device"]["identifiers"][0], format!("dspi_{SERIAL}"));
        assert_eq!(c["device"]["name"], "Front fills");
        assert_eq!(c["device"]["serial_number"], SERIAL);
        assert_eq!(c["device"]["sw_version"], "1.1.7");
        assert_eq!(c["device"]["model"], "RP2350");
        assert_eq!(c["availability"][0]["topic"], format!("{BASE}/status"));
        assert_eq!(
            c["availability"][1]["topic"],
            availability_topic(BASE, SERIAL)
        );
        assert_eq!(c["availability_mode"], "all");
        assert_eq!(c.get("command_topic").is_some(), e.settable(), "{e:?}");
    }
    let vol: Value =
        serde_json::from_str(&got[&config_topic(HUB, SERIAL, Entity::MasterVolume)]).unwrap();
    assert_eq!(
        (vol["min"].as_f64(), vol["max"].as_f64()),
        (Some(-128.0), Some(0.0))
    );
    assert_eq!(vol["unit_of_measurement"], "dB");
    let sel: Value =
        serde_json::from_str(&got[&config_topic(HUB, SERIAL, Entity::Preset)]).unwrap();
    assert_eq!(
        sel["options"],
        json!(["House", "Slot 1"]),
        "used slots only"
    );
    assert_eq!(got[&availability_topic(BASE, SERIAL)], "online");
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::MasterVolume)], "-20");
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Mute)], "OFF");
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Preset)], "House");
}

#[test]
fn wp_p2ha_facts_name_the_model() {
    let mut out = Outbox::default();
    let mut st = state_with_device(&mut out);
    out.drain();
    let identify = dspi_proto::ext::IdentifyStatus {
        remaining_s: 0,
        last_tone: None,
        chip: dspi_proto::ext::Chip::Rp2350,
        chip_rev: 2,
        chip_id: 0,
        board: "pico2".into(),
        hw_rev: "K340".into(),
    };
    st.on_bus(
        &mut out,
        &BusMessage::Facts {
            serial: SERIAL.into(),
            identify: Some(identify),
            reset_info: None,
            fw: "1.1.8".into(),
        },
    );
    let got = topics(&mut out);
    let c: Value = serde_json::from_str(&got[&config_topic(HUB, SERIAL, Entity::Cpu)]).unwrap();
    assert_eq!(c["device"]["model"], "pico2 (K340)");
    assert_eq!(c["device"]["sw_version"], "1.1.8");
    assert!(
        !got.contains_key(&state_topic(BASE, SERIAL, Entity::MasterVolume)),
        "an unchanged state is not published again"
    );
}

#[test]
fn wp_p2ha_state_follows_notifications_and_health() {
    let mut out = Outbox::default();
    let mut st = state_with_device(&mut out);
    out.drain();
    // The master volume's PARAM_CHANGED, and the user mute's.
    st.on_bus(
        &mut out,
        &notification(param_changed(
            g::OFF_MASTER_VOLUME,
            &(-31.5f32).to_le_bytes(),
        )),
    );
    st.on_bus(
        &mut out,
        &notification(param_changed(g::OFF_USER_VOLUME + 4, &[1])),
    );
    // A change elsewhere publishes nothing.
    st.on_bus(&mut out, &notification(param_changed(10, &[1, 2, 3, 4])));
    // A garbled packet is nothing too.
    st.on_bus(&mut out, &notification(vec![2, 0x7F, 0xFF]));
    let got = topics(&mut out);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        got[&state_topic(BASE, SERIAL, Entity::MasterVolume)],
        "-31.5"
    );
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Mute)], "ON");

    // A range that covers the volume: read from inside it.
    let mut bytes = vec![0u8; 8];
    bytes[2..6].copy_from_slice(&(-12.0f32).to_le_bytes());
    st.on_bus(
        &mut out,
        &notification(param_changed(g::OFF_MASTER_VOLUME - 2, &bytes)),
    );
    assert_eq!(
        topics(&mut out)[&state_topic(BASE, SERIAL, Entity::MasterVolume)],
        "-12"
    );

    // A preset loaded: its state, and a refresh for the volume it may carry.
    let preset_loaded = vec![2u8, g::notify::NOTIFY_EVT_PRESET_LOADED as u8, 0, 0, 1];
    let jobs = st.on_bus(&mut out, &notification(preset_loaded));
    assert_eq!(jobs, vec![Job::Refresh(SERIAL.into())]);
    assert_eq!(
        topics(&mut out)[&state_topic(BASE, SERIAL, Entity::Preset)],
        "Slot 1"
    );

    st.on_bus(
        &mut out,
        &BusMessage::Health {
            serial: SERIAL.into(),
            cpu: Some(42),
            clip_mask: 0b10,
            temp_c: Some(47.26),
            uptime_s: None,
            hang_count: None,
        },
    );
    let got = topics(&mut out);
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Temperature)], "47.3");
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Cpu)], "42");
    assert_eq!(got[&state_topic(BASE, SERIAL, Entity::Clip)], "ON");
    // A device it has not seen online is not made from health.
    st.on_bus(
        &mut out,
        &BusMessage::Health {
            serial: "OTHER".into(),
            cpu: Some(1),
            clip_mask: 0,
            temp_c: None,
            uptime_s: None,
            hang_count: None,
        },
    );
    assert!(out.is_empty());
    assert_eq!(st.serials(), vec![SERIAL.to_owned()]);
}

#[test]
fn wp_p2ha_commands_become_control_plane_messages() {
    let mut out = Outbox::default();
    let mut st = state_with_device(&mut out);
    out.drain();
    let job = st
        .command(
            &mut out,
            SERIAL,
            Entity::MasterVolume,
            Command::Volume(-25.0),
        )
        .unwrap();
    assert_eq!(
        job,
        Job::Write {
            serial: SERIAL.into(),
            entity: Entity::MasterVolume,
            msg: json!({"t": "param.set", "serial": SERIAL, "path": "vol.master",
                "idx": [], "value": -25.0}),
        }
    );
    let Job::Write { msg, .. } = st
        .command(&mut out, SERIAL, Entity::Mute, Command::Mute(true))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (msg["path"].clone(), msg["value"].clone()),
        (json!("vol.mute"), json!(true))
    );
    let Job::Write { msg, .. } = st
        .command(
            &mut out,
            SERIAL,
            Entity::Preset,
            Command::Preset("Slot 1".into()),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        msg,
        json!({"t": "preset.load", "serial": SERIAL, "slot": 1})
    );
    let Job::Write { msg, .. } = st
        .command(&mut out, SERIAL, Entity::Identify, Command::Identify)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        msg,
        json!({"t": "device.identify", "serial": SERIAL, "seconds": IDENTIFY_SECONDS})
    );
    assert!(out.is_empty());
    // A label it has not: nothing sent, the state put back.
    assert!(
        st.command(
            &mut out,
            SERIAL,
            Entity::Preset,
            Command::Preset("Nope".into())
        )
        .is_none()
    );
    assert_eq!(
        topics(&mut out)[&state_topic(BASE, SERIAL, Entity::Preset)],
        "House"
    );
    // A device without entities: nothing.
    assert!(
        st.command(&mut out, "OTHER", Entity::Mute, Command::Mute(true))
            .is_none()
    );
    // A refusal publishes the state as it is, even unchanged.
    st.on_feedback(
        &mut out,
        Feedback::Refused {
            serial: SERIAL.into(),
            entity: Entity::MasterVolume,
        },
    );
    assert_eq!(
        topics(&mut out)[&state_topic(BASE, SERIAL, Entity::MasterVolume)],
        "-20"
    );
}

#[test]
fn wp_p2ha_a_forgotten_device_is_removed_and_offline_is_unavailable() {
    // -- WP P2DFIXA -- forgotten is `inventory.forget`, not the end of the grace.
    let mut out = Outbox::default();
    let mut st = state_with_device(&mut out);
    out.drain();
    st.on_bus(
        &mut out,
        &BusMessage::DeviceOffline {
            serial: SERIAL.into(),
            handle: 1,
        },
    );
    assert_eq!(
        topics(&mut out)[&availability_topic(BASE, SERIAL)],
        "offline"
    );
    // -- WP P2DFIXA -- past the registry's grace: kept, unavailable.
    st.on_bus(
        &mut out,
        &BusMessage::DeviceRemoved {
            serial: SERIAL.into(),
            handle: 1,
        },
    );
    let got = topics(&mut out);
    assert!(
        got.keys().all(|t| !t.ends_with("/config")),
        "nothing removed: {got:?}"
    );
    assert_eq!(st.serials(), vec![SERIAL.to_owned()]);
    // Forgotten by an Admin (`inventory.forget`): removed.
    st.on_bus(
        &mut out,
        &BusMessage::DeviceForgotten {
            serial: SERIAL.into(),
        },
    );
    let got = topics(&mut out);
    for e in Entity::ALL {
        assert_eq!(got[&config_topic(HUB, SERIAL, e)], "", "{e:?} removed");
    }
    assert_eq!(got[&availability_topic(BASE, SERIAL)], "");
    assert!(st.serials().is_empty());
    // Back: discovered again.
    st.on_bus(&mut out, &online());
    assert!(topics(&mut out).contains_key(&config_topic(HUB, SERIAL, Entity::Mute)));
    // Turned off: everything removed.
    st.remove_all(&mut out);
    assert_eq!(
        topics(&mut out)[&config_topic(HUB, SERIAL, Entity::Mute)],
        ""
    );
}

#[test]
fn wp_p2ha_no_used_preset_no_select() {
    let mut out = Outbox::default();
    let mut st = state_with_device(&mut out);
    out.drain();
    st.on_feedback(
        &mut out,
        Feedback::Values {
            serial: SERIAL.into(),
            volume: None,
            mute: None,
            presets: Some(Vec::new()),
            active: Some(None),
        },
    );
    assert_eq!(
        topics(&mut out)[&config_topic(HUB, SERIAL, Entity::Preset)],
        "",
        "the select is removed"
    );
}

#[test]
fn wp_p2ha_preset_labels_are_unique_and_short() {
    let long = "x".repeat(100);
    let labels = preset_labels(&[
        (0, "A".into(), true),
        (1, "A".into(), true),
        (2, "  ".into(), true),
        (3, long.clone(), true),
        (4, long, true),
        (5, "B".into(), false),
    ]);
    let names: Vec<&str> = labels.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(&names[..3], ["A", "A (slot 1)", "Slot 2"]);
    assert!(labels.iter().all(|(_, l)| l.chars().count() <= 64));
    assert_ne!(labels[3].1, labels[4].1);
    assert_eq!(labels.len(), 5);
}

#[test]
fn wp_p2ha_the_outbox_keeps_the_latest_per_topic_and_is_bounded() {
    let mut out = Outbox::default();
    out.put("a".into(), b"1".to_vec());
    out.put("b".into(), b"1".to_vec());
    out.put("a".into(), b"2".to_vec());
    assert_eq!(
        out.drain(),
        vec![("a".into(), b"2".to_vec()), ("b".into(), b"1".to_vec())]
    );
    for i in 0..MAX_OUTBOX + 10 {
        out.put(format!("t{i}"), Vec::new());
    }
    assert_eq!(out.len(), MAX_OUTBOX);
    assert_eq!(out.dropped(), 10);
    assert_eq!(out.pop().unwrap().0, "t10", "the oldest went");
}

#[test]
fn wp_p2ha_the_inbound_queue_is_bounded_and_drops_retained_and_long_messages() {
    let ha = HaShared::new(HaSetting::default());
    let msg = |topic: &str, payload: &[u8], retain: bool| MqttEvent::Message {
        topic: topic.into(),
        payload: payload.to_vec(),
        retain,
    };
    // Off: nothing kept.
    ha.offer(&msg("t", b"ON", false));
    assert!(ha.take().is_empty());
    ha.set(HaSetting {
        enabled: true,
        key: Some(3),
    });
    ha.offer(&msg("t", b"ON", true));
    assert!(ha.take().is_empty(), "a retained set is never replayed");
    ha.offer(&msg("t", &[b'x'; MAX_PAYLOAD + 1], false));
    ha.offer(&msg(&"t".repeat(MAX_TOPIC + 1), b"ON", false));
    assert!(ha.take().is_empty());
    assert_eq!(ha.dropped(), 2);
    for i in 0..INBOUND_CAP + 5 {
        ha.offer(&msg(&format!("t{i}"), b"ON", false));
    }
    let got = ha.take();
    assert_eq!(got.len(), INBOUND_CAP);
    assert_eq!(got[0].0, "t5", "the oldest went");
    assert_eq!(ha.dropped(), 7);
    ha.offer(&MqttEvent::Connected {
        session_present: false,
    });
    assert!(ha.take().is_empty());
}

#[test]
fn wp_p2ha_a_newer_set_replaces_one_still_waiting() {
    let w = |entity: Entity, v: f64| Job::Write {
        serial: SERIAL.into(),
        entity,
        msg: json!({"value": v}),
    };
    assert!(supersedes(
        &w(Entity::MasterVolume, 1.0),
        &w(Entity::MasterVolume, 2.0)
    ));
    assert!(!supersedes(
        &w(Entity::MasterVolume, 1.0),
        &w(Entity::Mute, 2.0)
    ));
    assert!(!supersedes(
        &w(Entity::Identify, 1.0),
        &w(Entity::Identify, 1.0)
    ));
    assert!(supersedes(
        &Job::Refresh(SERIAL.into()),
        &Job::Refresh(SERIAL.into())
    ));
    assert!(!supersedes(
        &Job::Refresh("A".into()),
        &Job::Refresh("B".into())
    ));
}
