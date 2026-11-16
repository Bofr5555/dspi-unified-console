//! WP P3RO: `rollout_failed`, raised critical from `RolloutEnded`, cleared by a later
//! rollout that ends ok or by the device attaching with the image's firmware (not after
//! a failed verify), and never turned off by the rules.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;

const S: &str = "P3RO-UNIT";

fn now() -> Now {
    Now {
        mono: Instant::now(),
        wall_s: 1_800_000_000,
    }
}

fn book() -> DeviceAlarms {
    DeviceAlarms::with_floor(Rules::default(), now(), 1_700_000_000)
}

fn ended(ok: bool, step: &str) -> BusMessage {
    BusMessage::RolloutEnded {
        serial: S.into(),
        ok,
        step: step.into(),
        text: "no board appeared in BOOTSEL".into(),
        firmware_version: "1.2.0".into(),
    }
}

fn facts(fw: &str) -> BusMessage {
    BusMessage::Facts {
        serial: S.into(),
        identify: None,
        reset_info: None,
        fw: fw.into(),
    }
}

fn views(effects: &[Effect]) -> Vec<AlarmView> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Changed(v) => Some(v.clone()),
            Effect::Mute { .. } => None,
        })
        .collect()
}

#[test]
fn wp_p3ro_a_failed_rollout_raises_a_critical_alarm_with_its_step() {
    let mut a = book();
    let up = views(&a.on_message(&ended(false, "find"), now()));
    assert_eq!(up.len(), 1);
    assert_eq!(up[0].code, ROLLOUT_FAILED);
    assert_eq!(up[0].severity, "critical");
    assert_eq!(up[0].serial.as_deref(), Some(S));
    assert_eq!(up[0].state, AlarmState::Raised);
    assert!(up[0].message.contains("at find"), "{}", up[0].message);
    // A later rollout of the device that ends ok clears it.
    let down = views(&a.on_message(&ended(true, "verify"), now()));
    assert_eq!(down.len(), 1);
    assert_eq!(down[0].state, AlarmState::Cleared);
    assert!(!a.is_on(ROLLOUT_FAILED, S));
}

#[test]
fn wp_p3ro_the_device_back_with_the_image_clears_it_but_not_after_a_failed_verify() {
    let mut a = book();
    a.on_message(&ended(false, "wait"), now());
    assert!(views(&a.on_message(&facts("1.1.0"), now())).is_empty());
    assert!(a.is_on(ROLLOUT_FAILED, S));
    let down = views(&a.on_message(&facts("1.2.0"), now()));
    assert_eq!(down[0].state, AlarmState::Cleared);

    // A verify failure (the core voltage read back wrong) is not cleared by a replug.
    a.on_message(&ended(false, "verify"), now());
    assert!(views(&a.on_message(&facts("1.2.0"), now())).is_empty());
    assert!(a.is_on(ROLLOUT_FAILED, S));
    // Acknowledging it keeps it on.
    let id = a.list()[0]
        .id
        .trim_start_matches("device:")
        .parse()
        .unwrap();
    a.acknowledge(id, Some("Bench".into()), now()).unwrap();
    assert!(a.is_on(ROLLOUT_FAILED, S));
}

#[test]
fn wp_p3ro_the_rules_cannot_turn_it_off() {
    let mut a = book();
    a.on_message(&ended(false, "flash"), now());
    let mut off = serde_json::Map::new();
    for code in CODES {
        off.insert(code.into(), json!({"on": false}));
    }
    let rules = a.rules().merged(&Value::Object(off)).unwrap();
    a.set_rules(rules, now());
    assert!(a.is_on(ROLLOUT_FAILED, S));
    // And it is raised with every rule off.
    let mut b = book();
    let rules = b
        .rules()
        .merged(&json!({"rollout_failed": {"on": false}}))
        .unwrap();
    b.set_rules(rules, now());
    assert_eq!(views(&b.on_message(&ended(false, "find"), now())).len(), 1);
}
