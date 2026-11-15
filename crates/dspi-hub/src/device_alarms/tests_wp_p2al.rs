//! WP P2AL's done criteria. The pure state with a test clock: each code raised and
//! cleared, a latch-only device's "clipped" once until cleared, an acknowledged alarm on
//! until cleared, the mute once per storm and never an unmute, the rules. Then through an
//! in-process hub on the demo device: the mute passes another session's lock and is
//! recorded with the rule's name, the rules are an admin's, the audio link's alarms are
//! listed and acknowledged beside the device ones.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::SystemTime;

use dspi_proto::ext::{Chip, ResetInfo};
use dspi_session::notify::PROTOCOL_VERSION;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, Transport,
    TransportCaps,
};

use super::*;
use crate::bus::TargetResult;
use crate::clock::TestClock;
use crate::hub::{Hub, HubConfig};

const S: &str = "P2AL-UNIT";
const T0: u64 = 1_800_000_000;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/<branch>, not tmpfs.
    let dir = std::env::temp_dir().join("wp-P2AL").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Time(Now);

impl Time {
    fn new() -> Self {
        Self(Now {
            mono: Instant::now(),
            wall_s: T0,
        })
    }
    fn now(&self) -> Now {
        self.0
    }
    fn advance(&mut self, s: u64) {
        self.0.mono += Duration::from_secs(s);
        self.0.wall_s += s;
    }
}

fn book(t: &Time) -> DeviceAlarms {
    DeviceAlarms::with_floor(Rules::default(), t.now(), T0 - 86_400)
}

fn changed(effects: &[Effect]) -> Vec<(String, AlarmState)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Changed(v) => Some((v.code.clone(), v.state)),
            Effect::Mute { .. } => None,
        })
        .collect()
}

fn mutes(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Mute { .. }))
        .count()
}

fn tick(a: &mut DeviceAlarms, t: &Time) -> Vec<Effect> {
    let mut out = Vec::new();
    a.tick(t.now(), &mut out);
    out
}

fn online(serial: &str) -> BusMessage {
    BusMessage::DeviceOnline {
        serial: serial.into(),
        handle: 0,
    }
}

fn offline(serial: &str) -> BusMessage {
    BusMessage::DeviceOffline {
        serial: serial.into(),
        handle: 0,
    }
}

fn facts(serial: &str, reset: Option<(ResetReason, u32)>, ext: bool) -> BusMessage {
    BusMessage::Facts {
        serial: serial.into(),
        identify: None,
        reset_info: reset
            .map(|(reason, uptime_s)| ResetInfo {
                reason,
                hang_count: 0,
                chip: Chip::Rp2350,
                uptime_s,
                prev_uptime_s: None,
                chip_reset: 0,
                watchdog_reason: 0,
            })
            .or_else(|| {
                ext.then_some(ResetInfo {
                    reason: ResetReason::PowerOn,
                    hang_count: 0,
                    chip: Chip::Rp2350,
                    uptime_s: 100_000,
                    prev_uptime_s: None,
                    chip_reset: 0,
                    watchdog_reason: 0,
                })
            }),
        fw: "1.1.7".into(),
    }
}

fn health(serial: &str, cpu: Option<u8>, clip_mask: u32, temp_c: Option<f32>) -> BusMessage {
    BusMessage::Health {
        serial: serial.into(),
        cpu,
        clip_mask,
        temp_c,
        uptime_s: None,
        hang_count: None,
    }
}

/// An extension clip event (0xE0) on channel 0.
fn clip_packet(seq: u8) -> Vec<u8> {
    let mut p = vec![PROTOCOL_VERSION, NOTIFY_EVT_EXT_CLIP, 0, seq, 0, 1, 0, 0];
    p.extend(1u32.to_le_bytes());
    p
}

fn clip(serial: &str, seq: u8) -> BusMessage {
    BusMessage::Notification {
        serial: serial.into(),
        session: 0,
        by: None,
        packet: Arc::from(clip_packet(seq)),
    }
}

// ── Each code raised and cleared ────────────────────────────────────────────

#[test]
fn wp_p2al_device_offline_after_30_s_and_cleared_when_back() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    assert!(a.on_message(&offline(S), t.now()).is_empty());
    t.advance(29);
    assert!(changed(&tick(&mut a, &t)).is_empty(), "not yet");
    t.advance(1);
    let up = tick(&mut a, &t);
    assert_eq!(
        changed(&up),
        [("device_offline".into(), AlarmState::Raised)]
    );
    let Effect::Changed(v) = &up[0] else { panic!() };
    assert_eq!(v.id, "device:1");
    assert_eq!(v.serial.as_deref(), Some(S));
    assert_eq!(v.severity, "critical");
    assert!(tick(&mut a, &t).is_empty(), "raised once");
    let back = a.on_message(&online(S), t.now());
    assert_eq!(
        changed(&back),
        [("device_offline".into(), AlarmState::Cleared)]
    );
    // Back within the time: nothing at all.
    a.on_message(&offline(S), t.now());
    t.advance(10);
    a.on_message(&online(S), t.now());
    t.advance(60);
    assert!(tick(&mut a, &t).is_empty());
}

#[test]
fn wp_p2al_unresponsive_raised_after_its_time_and_cleared_by_a_reading() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    // P2HP reports after 10 s of silence: at once past the default 10 s.
    let up = a.on_message(
        &BusMessage::Unresponsive {
            serial: S.into(),
            since: T0 - 10,
        },
        t.now(),
    );
    assert_eq!(changed(&up), [("unresponsive".into(), AlarmState::Raised)]);
    t.advance(2);
    let down = a.on_message(&health(S, Some(10), 0, None), t.now());
    assert_eq!(
        changed(&down),
        [("unresponsive".into(), AlarmState::Cleared)]
    );

    // A longer rule waits for its time; an answer before it is no alarm.
    a.set_rules(
        Rules::default()
            .merged(&json!({"unresponsive": {"after_s": 30}}))
            .unwrap(),
        t.now(),
    );
    a.on_message(
        &BusMessage::Unresponsive {
            serial: S.into(),
            since: t.now().wall_s - 10,
        },
        t.now(),
    );
    t.advance(19);
    assert!(tick(&mut a, &t).is_empty());
    t.advance(1);
    assert_eq!(
        changed(&tick(&mut a, &t)),
        [("unresponsive".into(), AlarmState::Raised)]
    );
    // Detached: over (the offline alarm takes it from here).
    let gone = a.on_message(&offline(S), t.now());
    assert_eq!(
        changed(&gone),
        [("unresponsive".into(), AlarmState::Cleared)]
    );
}

#[test]
fn wp_p2al_a_watchdog_restart_is_raised_and_cleared_by_acknowledging() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    // Power-on and software resets are events, never alarms.
    for reason in [
        ResetReason::PowerOn,
        ResetReason::Software,
        ResetReason::RunPin,
    ] {
        assert!(
            a.on_message(&facts(S, Some((reason, 5)), true), t.now())
                .is_empty()
        );
    }
    let up = a.on_message(&facts(S, Some((ResetReason::Watchdog, 5)), true), t.now());
    assert_eq!(
        changed(&up),
        [("device_restarted".into(), AlarmState::Raised)]
    );
    let Effect::Changed(v) = &up[0] else { panic!() };
    assert!(v.message.contains("watchdog"), "{}", v.message);
    assert!(v.message.starts_with("Restarted at "), "{}", v.message);
    // The same restart seen again (a re-attach) is not a second one.
    t.advance(30);
    a.on_message(&offline(S), t.now());
    a.on_message(&online(S), t.now());
    assert!(
        a.on_message(&facts(S, Some((ResetReason::Watchdog, 35)), true), t.now())
            .is_empty()
    );
    // A new one counts again.
    t.advance(100);
    let again = a.on_message(&facts(S, Some((ResetReason::Glitch, 3)), true), t.now());
    let Effect::Changed(v) = &again[0] else {
        panic!()
    };
    assert_eq!(v.count, 2);
    // Acknowledged, it is over.
    let n = 1;
    let acked = a
        .acknowledge(n, Some("Front of house".into()), t.now())
        .unwrap();
    assert_eq!(
        changed(&acked),
        [
            ("device_restarted".into(), AlarmState::Acknowledged),
            ("device_restarted".into(), AlarmState::Cleared)
        ]
    );
    let Effect::Changed(v) = &acked[1] else {
        panic!()
    };
    assert_eq!(v.acknowledged_by.as_deref(), Some("Front of house"));
    assert!(!a.is_on("device_restarted", S));
}

#[test]
fn wp_p2al_an_old_restart_seen_on_the_first_attach_is_no_alarm() {
    let mut t = Time::new();
    t.advance(5);
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    // A day of uptime since a watchdog reset, long before this run: history, not news.
    assert!(
        a.on_message(
            &facts(S, Some((ResetReason::Brownout, 86_400)), true),
            t.now()
        )
        .is_empty()
    );
    // Five minutes ago, before the hub started: still news.
    let up = a.on_message(
        &facts("P2AL-OTHER", Some((ResetReason::Brownout, 300)), true),
        t.now(),
    );
    assert_eq!(
        changed(&up),
        [("device_restarted".into(), AlarmState::Raised)]
    );
}

#[test]
fn wp_p2al_a_clip_storm_is_raised_at_the_rate_and_cleared_after_a_quiet_minute() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    // Before its facts nobody knows 0xE0 is the extension's: nothing.
    assert!(a.on_message(&clip(S, 0), t.now()).is_empty());
    a.on_message(&facts(S, None, true), t.now());
    let mut raised = Vec::new();
    for n in 1..=9 {
        raised.extend(a.on_message(&clip(S, n), t.now()));
        t.advance(5);
    }
    assert!(raised.is_empty(), "9 in a minute is under 10");
    let up = a.on_message(&clip(S, 10), t.now());
    assert_eq!(changed(&up), [("clip".into(), AlarmState::Raised)]);
    assert_eq!(mutes(&up), 0, "the mute is off by default");
    // The latch of extension firmware is left to the events.
    assert!(a.on_message(&health(S, None, 0, None), t.now()).is_empty());
    // A minute under the rate (the window empties, then a minute): cleared.
    t.advance(60);
    assert!(tick(&mut a, &t).is_empty());
    t.advance(59);
    assert!(tick(&mut a, &t).is_empty());
    t.advance(1);
    assert_eq!(
        changed(&tick(&mut a, &t)),
        [("clip".into(), AlarmState::Cleared)]
    );
}

#[test]
fn wp_p2al_a_latch_only_device_is_clipped_once_until_the_latch_is_cleared() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    a.on_message(&facts(S, None, false), t.now());
    let up = a.on_message(&health(S, Some(10), 0b101, None), t.now());
    assert_eq!(changed(&up), [("clip".into(), AlarmState::Raised)]);
    let Effect::Changed(v) = &up[0] else { panic!() };
    assert!(v.message.contains("channels 1, 3"), "{}", v.message);
    for _ in 0..20 {
        t.advance(2);
        assert!(
            a.on_message(&health(S, Some(10), 0b111, None), t.now())
                .is_empty(),
            "once"
        );
    }
    // Clip events from such firmware are not read (0xE0 is not its).
    assert!(a.on_message(&clip(S, 1), t.now()).is_empty());
    // Acknowledged it stays on; a person clearing the latch clears it.
    let acked = a.acknowledge(1, None, t.now()).unwrap();
    assert_eq!(changed(&acked), [("clip".into(), AlarmState::Acknowledged)]);
    t.advance(2);
    assert!(
        a.on_message(&health(S, Some(10), 0b101, None), t.now())
            .is_empty()
    );
    assert!(a.is_on("clip", S));
    let down = a.on_message(&health(S, Some(10), 0, None), t.now());
    assert_eq!(changed(&down), [("clip".into(), AlarmState::Cleared)]);
    // A clip after that is new again.
    let again = a.on_message(&health(S, Some(10), 0b1, None), t.now());
    assert_eq!(changed(&again), [("clip".into(), AlarmState::Raised)]);
}

#[test]
fn wp_p2al_temperature_with_its_hysteresis_and_acknowledged_stays_on() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    assert!(
        a.on_message(&health(S, None, 0, Some(69.9)), t.now())
            .is_empty()
    );
    let up = a.on_message(&health(S, None, 0, Some(70.0)), t.now());
    assert_eq!(changed(&up), [("temperature".into(), AlarmState::Raised)]);
    let acked = a.acknowledge(1, Some("Viewer".into()), t.now()).unwrap();
    assert_eq!(
        changed(&acked),
        [("temperature".into(), AlarmState::Acknowledged)]
    );
    // Acknowledged again: no change, the same object.
    let twice = a.acknowledge(1, None, t.now()).unwrap();
    assert_eq!(
        changed(&twice),
        [("temperature".into(), AlarmState::Acknowledged)]
    );
    for c in [72.0, 68.0, 65.1] {
        t.advance(2);
        assert!(
            a.on_message(&health(S, None, 0, Some(c)), t.now())
                .is_empty()
        );
    }
    let listed = a.list();
    assert_eq!(listed[0].state, AlarmState::Acknowledged, "still on");
    assert_eq!(listed[0].acknowledged_by.as_deref(), Some("Viewer"));
    let down = a.on_message(&health(S, None, 0, Some(64.9)), t.now());
    assert_eq!(
        changed(&down),
        [("temperature".into(), AlarmState::Cleared)]
    );
    // NaN is no reading.
    assert!(
        a.on_message(&health(S, None, 0, Some(f32::NAN)), t.now())
            .is_empty()
    );
    // Gone, nothing is left on that a running device has.
    a.on_message(&health(S, None, 0, Some(90.0)), t.now());
    let gone = a.on_message(&offline(S), t.now());
    assert_eq!(
        changed(&gone),
        [("temperature".into(), AlarmState::Cleared)]
    );
}

#[test]
fn wp_p2al_cpu_over_for_ten_seconds() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.on_message(&online(S), t.now());
    for _ in 0..5 {
        assert!(
            a.on_message(&health(S, Some(95), 0, None), t.now())
                .is_empty()
        );
        t.advance(2);
    }
    // A dip resets it.
    assert!(
        a.on_message(&health(S, Some(50), 0, None), t.now())
            .is_empty()
    );
    let mut got = Vec::new();
    for _ in 0..6 {
        got.extend(a.on_message(&health(S, Some(90), 0, None), t.now()));
        t.advance(2);
    }
    assert_eq!(changed(&got), [("cpu".into(), AlarmState::Raised)]);
    let down = a.on_message(&health(S, Some(89), 0, None), t.now());
    assert_eq!(changed(&down), [("cpu".into(), AlarmState::Cleared)]);
}

#[test]
fn wp_p2al_a_failed_schedule_is_raised_counted_and_cleared_by_acknowledging() {
    let t = Time::new();
    let mut a = book(&t);
    let ran = |ok: bool| BusMessage::ScheduleRan {
        rule: "r1".into(),
        name: "Doors open".into(),
        at: T0,
        by: None,
        results: vec![
            TargetResult {
                serial: "A".into(),
                action: Some("0:preset.load".into()),
                ok: true,
                code: None,
                msg: None,
            },
            TargetResult {
                serial: "B".into(),
                action: Some("0:preset.load".into()),
                ok,
                code: (!ok).then(|| "offline".into()),
                msg: None,
            },
        ],
    };
    assert!(a.on_message(&ran(true), t.now()).is_empty());
    let up = a.on_message(&ran(false), t.now());
    assert_eq!(
        changed(&up),
        [("schedule_failed".into(), AlarmState::Raised)]
    );
    let Effect::Changed(v) = &up[0] else { panic!() };
    assert_eq!(v.subject, "r1");
    assert_eq!(v.serial, None, "about a rule, not a device");
    assert!(v.message.contains("1 of 2"), "{}", v.message);
    assert!(v.message.contains("B offline"), "{}", v.message);
    let again = a.on_message(&ran(false), t.now());
    let Effect::Changed(v) = &again[0] else {
        panic!()
    };
    assert_eq!(v.count, 2);
    let acked = a.acknowledge(1, None, t.now()).unwrap();
    assert_eq!(
        changed(&acked).last().unwrap(),
        &("schedule_failed".into(), AlarmState::Cleared)
    );
}

#[test]
fn wp_p2al_clock_unset_while_the_clock_reads_before_the_build() {
    let mut t = Time::new();
    let mut a = DeviceAlarms::with_floor(Rules::default(), t.now(), T0 + 100);
    let up = tick(&mut a, &t);
    assert_eq!(changed(&up), [("clock_unset".into(), AlarmState::Raised)]);
    let Effect::Changed(v) = &up[0] else { panic!() };
    assert_eq!(v.subject, "hub");
    assert_eq!(v.serial, None);
    assert!(tick(&mut a, &t).is_empty());
    t.advance(100);
    assert_eq!(
        changed(&tick(&mut a, &t)),
        [("clock_unset".into(), AlarmState::Cleared)]
    );
    // The real floor is this source's date.
    assert_eq!(iso8601(BUILD_FLOOR_S), "2026-11-15T00:00:00Z");
}

// ── The mute on a clip storm ────────────────────────────────────────────────

#[test]
fn wp_p2al_the_mute_comes_once_per_storm_and_never_an_unmute() {
    let mut t = Time::new();
    let mut a = book(&t);
    a.set_rules(
        Rules::default()
            .merged(&json!({"clip": {"mute_on_storm": true}}))
            .unwrap(),
        t.now(),
    );
    a.on_message(&online(S), t.now());
    a.on_message(&facts(S, None, true), t.now());
    let mut first = Vec::new();
    for n in 0..30 {
        first.extend(a.on_message(&clip(S, n), t.now()));
        t.advance(1);
    }
    assert_eq!(mutes(&first), 1, "once in the storm: {first:?}");
    let raised = first
        .iter()
        .find_map(|e| match e {
            Effect::Changed(v) => Some(v.clone()),
            Effect::Mute { .. } => None,
        })
        .unwrap();
    assert!(
        raised.message.contains("the hub muted"),
        "{}",
        raised.message
    );
    // The storm ends; nothing is unmuted (there is no such effect at all).
    t.advance(61);
    assert!(
        tick(&mut a, &t).is_empty(),
        "under the rate: a minute begins"
    );
    t.advance(60);
    let calm = tick(&mut a, &t);
    assert_eq!(changed(&calm), [("clip".into(), AlarmState::Cleared)]);
    assert_eq!(mutes(&calm), 0);
    // A new storm mutes again, once.
    let mut second = Vec::new();
    for n in 0..20 {
        second.extend(a.on_message(&clip(S, n), t.now()));
    }
    assert_eq!(mutes(&second), 1);
    // A rule turned off clears its alarm and mutes nothing more.
    let off = a.set_rules(
        Rules::default()
            .merged(&json!({"clip": {"on": false, "mute_on_storm": true}}))
            .unwrap(),
        t.now(),
    );
    assert_eq!(changed(&off), [("clip".into(), AlarmState::Cleared)]);
    let mut more = Vec::new();
    for n in 0..20 {
        more.extend(a.on_message(&clip(S, n), t.now()));
    }
    assert!(more.is_empty(), "{more:?}");
}

// ── The rules ───────────────────────────────────────────────────────────────

#[test]
fn wp_p2al_the_default_rules_are_the_plans() {
    let r = Rules::default();
    for code in CODES {
        assert!(r.on(code), "{code}");
    }
    assert_eq!(r.int("device_offline", "after_s"), 30);
    assert_eq!(r.int("unresponsive", "after_s"), 10);
    assert_eq!(r.int("clip", "per_minute"), 10);
    assert!(!r.flag("clip", "mute_on_storm"));
    assert_eq!(r.num("temperature", "above_c"), 70.0);
    assert_eq!(r.int("cpu", "above_pct"), 90);
    assert_eq!(r.int("cpu", "for_s"), 10);
    assert_eq!(
        r.to_json()["clip"],
        json!({"on": true, "per_minute": 10, "mute_on_storm": false})
    );
}

#[test]
fn wp_p2al_rules_merge_in_part_and_refuse_a_bad_value_whole() {
    let r = Rules::default();
    let m = r
        .merged(
            &json!({"temperature": {"above_c": 75.5}, "cpu": {"on": false},
            "later_code": {"on": false}, "clip": {"per_minute": 20, "new_field": 1}}),
        )
        .unwrap();
    assert_eq!(m.num("temperature", "above_c"), 75.5);
    assert!(m.on("temperature"), "a field left out keeps its value");
    assert!(!m.on("cpu"));
    assert_eq!(m.int("cpu", "above_pct"), 90);
    assert_eq!(m.int("clip", "per_minute"), 20);
    assert!(m.on("device_offline"), "a code left out keeps its rule");
    assert!(m.to_json().get("later_code").is_none());
    for bad in [
        json!([]),
        json!({"cpu": 5}),
        json!({"cpu": {"on": "yes"}}),
        json!({"cpu": {"above_pct": 101}}),
        json!({"cpu": {"above_pct": 0}}),
        json!({"cpu": {"for_s": 1.5}}),
        json!({"temperature": {"above_c": -1}}),
        json!({"device_offline": {"after_s": 0}}),
        json!({"clip": {"mute_on_storm": 1}}),
        json!({"temperature": {"above_c": 72}, "unresponsive": {"after_s": -3}}),
    ] {
        assert!(r.merged(&bad).is_err(), "{bad}");
    }
}

#[test]
fn wp_p2al_a_torn_rules_file_keeps_what_reads_and_is_kept_aside() {
    let dir = scratch("torn");
    let path = dir.join(FILE_NAME);
    assert_eq!(load(&path, T0), Rules::default(), "none: the defaults");
    std::fs::write(
        &path,
        r#"{"v":1,"rules":{"temperature":{"on":true,"above_c":60},"cpu":{"above_pct":"x"},"clip":{"mute_on_storm":true}}}"#,
    )
    .unwrap();
    let r = load(&path, T0);
    assert_eq!(r.num("temperature", "above_c"), 60.0);
    assert!(r.flag("clip", "mute_on_storm"));
    assert_eq!(r.int("cpu", "above_pct"), 90, "the bad entry's default");
    assert!(dir.join(format!("{FILE_NAME}.bad-{T0}")).exists());
    // Written back whole.
    let back = std::fs::read_to_string(&path).unwrap();
    assert_eq!(Rules::from_file(&back), (r.clone(), true));
    // Not JSON at all, or too large: the defaults.
    std::fs::write(&path, "{\"rules\": {").unwrap();
    assert_eq!(load(&path, T0 + 1), Rules::default());
    std::fs::write(&path, " ".repeat(70 * 1024)).unwrap();
    assert_eq!(load(&path, T0 + 2), Rules::default());
    assert!(dir.join(format!("{FILE_NAME}.bad-{}", T0 + 2)).exists());
}

#[test]
fn wp_p2al_no_panic_on_hostile_bus_values() {
    let t = Time::new();
    let mut a = book(&t);
    let long = "x".repeat(10_000);
    for m in [
        online(""),
        online(&long),
        health(&long, Some(255), u32::MAX, Some(f32::INFINITY)),
        BusMessage::Unresponsive {
            serial: S.into(),
            since: u64::MAX,
        },
        BusMessage::Unresponsive {
            serial: S.into(),
            since: 0,
        },
        BusMessage::Notification {
            serial: S.into(),
            session: 0,
            by: None,
            packet: Arc::from(vec![PROTOCOL_VERSION, NOTIFY_EVT_EXT_CLIP]),
        },
        facts(S, Some((ResetReason::Watchdog, u32::MAX)), true),
        BusMessage::ScheduleRan {
            rule: long.clone(),
            name: long.clone(),
            at: 0,
            by: None,
            results: Vec::new(),
        },
    ] {
        let _ = a.on_message(&m, t.now());
    }
    assert!(a.devices.len() <= 2, "no empty or overlong serial is kept");
    assert!(a.acknowledge(999, None, t.now()).is_err());
    // Bounded devices.
    for n in 0..(MAX_DEVICES + 50) {
        let s = format!("D{n}");
        a.on_message(&online(&s), t.now());
        a.on_message(&offline(&s), t.now());
    }
    assert!(a.devices.len() <= MAX_DEVICES);
}

#[test]
fn wp_p2al_the_bus_filter_takes_only_what_the_alarms_read() {
    assert!(wanted(&online(S)));
    assert!(wanted(&clip(S, 0)));
    assert!(!wanted(&BusMessage::Notification {
        serial: S.into(),
        session: 0,
        by: None,
        packet: Arc::from(vec![PROTOCOL_VERSION, 0x01, 0, 0]),
    }));
    let v = AlarmView::device(
        &Alarm {
            id: 1,
            code: "clip".into(),
            subject: S.into(),
            severity: "warning".into(),
            message: "m".into(),
            state: AlarmState::Raised,
            raised_at: T0,
            cleared_at: None,
            count: 1,
        },
        None,
    );
    assert!(!wanted(&v.to_bus(T0)), "its own alarms are not read back");
    assert_eq!(clip_event(&clip_packet(1)), Some(1));
    assert_eq!(clip_event(&[1, 2, 3]), None);
}

#[test]
fn wp_p2al_the_alarm_object_has_the_documented_shape() {
    let a = Alarm {
        id: 4,
        code: "temperature".into(),
        subject: S.into(),
        severity: "warning".into(),
        message: "72.0 °C".into(),
        state: AlarmState::Cleared,
        raised_at: T0,
        cleared_at: Some(T0 + 60),
        count: 2,
    };
    let v = AlarmView::device(&a, Some(&"Front of house".to_owned()));
    assert_eq!(
        v.to_json(),
        json!({"id": "device:4", "source": "device", "code": "temperature", "subject": S,
            "serial": S, "severity": "critical", "message": "72.0 °C", "state": "cleared",
            "raised_at": iso8601(T0), "cleared_at": iso8601(T0 + 60),
            "acknowledged_by": "Front of house", "count": 2})
    );
    let l = AlarmView::alink(&a, None).to_json();
    assert_eq!(l["id"], "alink:4");
    assert_eq!(l["source"], "alink");
    assert_eq!(l["severity"], "warning", "the audio link book's own");
    assert!(l.get("serial").is_none());
    assert_eq!(alarm_id(Some(&json!("device:12"))).unwrap(), ("device", 12));
    assert_eq!(alarm_id(Some(&json!("alink:3"))).unwrap(), ("alink", 3));
    for bad in [
        json!(12),
        json!("12"),
        json!("other:1"),
        json!("device:x"),
        json!(null),
    ] {
        assert_eq!(alarm_id(Some(&bad)).unwrap_err().code, "bad_request");
    }
}

// ── Through the hub, on the demo device ─────────────────────────────────────

/// One transfer that reached the device: `(dir, opcode, wValue, payload)`.
type Transfer = (Dir, u8, u16, Vec<u8>);

#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<Transfer>>>);

impl Wire {
    fn user_mutes(&self) -> Vec<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, o, _, _)| *d == Dir::Set && *o == op::REQ_SET_USER_MUTE)
            .map(|(_, _, _, p)| p.clone())
            .collect()
    }
}

struct Probe {
    inner: FakeDevice,
    wire: Wire,
    descriptor: DeviceDescriptor,
}

impl Transport for Probe {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.wire
            .0
            .lock()
            .unwrap()
            .push((Dir::Set, o, v, d.to_vec()));
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

struct Rig {
    hub: Hub,
    clock: Arc<TestClock>,
    dir: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        let dir = scratch(name);
        let clock = Arc::new(TestClock::at(
            SystemTime::UNIX_EPOCH + Duration::from_secs(T0),
        ));
        let hub = Hub::start(HubConfig {
            scan_interval: Duration::from_millis(10),
            data_dir: Some(dir.clone()),
            services: ServiceConfig {
                clock: Arc::clone(&clock) as Arc<dyn HubClock>,
                alarms: true,
                history: Some(crate::history::HistoryConfig::default()),
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("P2AL hub")
        })
        .unwrap();
        Self { hub, clock, dir }
    }

    fn plug(&self, serial: &str) -> (u8, Wire) {
        let wire = Wire::default();
        let probe = Probe {
            inner: FakeDevice::new(
                Capture::demo(),
                FakeOptions {
                    extensions: true,
                    diag_ext: true,
                    ..FakeOptions::default()
                },
            ),
            wire: wire.clone(),
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
            },
        };
        (self.hub.attach(Box::new(probe), 0).unwrap(), wire)
    }

    fn session(&self, role: Role, name: &str) -> Arc<Session> {
        self.hub.handle().open_principal(role, None, name).unwrap()
    }

    fn ask(&self, session: &Arc<Session>, msg: Value) -> Value {
        let text = self
            .hub
            .handle()
            .service_message(session, &msg)
            .expect("the alarms answer");
        serde_json::from_str(&text).unwrap()
    }

    /// Wait (real time, bounded) until `done` holds, waking the services meanwhile.
    fn until(&self, what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "no {what} in time"
            );
            self.hub.handle().wake_services();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

#[test]
fn wp_p2al_the_mute_passes_another_sessions_lock_once_per_storm_and_is_recorded() {
    let rig = Rig::new("mute");
    let serial = "P2AL-MUTE";
    let (handle, wire) = rig.plug(serial);
    let hub = rig.hub.handle();
    let admin = rig.session(Role::Admin, "Installer");
    let set = rig.ask(
        &admin,
        json!({"t": "alarm.rules.set", "id": 1, "rules": {"clip": {"mute_on_storm": true}}}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    assert_eq!(set["rules"]["clip"]["mute_on_storm"], true);
    // Another session holds the device's lock.
    let holder = rig.session(Role::Control, "Holder");
    assert!(hub.lock_acquire(&holder, handle, Duration::from_secs(300)));
    // Extension firmware (its facts, as P2HP sends them), then a storm.
    hub.bus().publish(facts(serial, None, true));
    for n in 0..12 {
        hub.bus().publish(clip(serial, n));
    }
    rig.until("mute", || !wire.user_mutes().is_empty());
    assert_eq!(wire.user_mutes(), vec![vec![1]], "a mute, past the lock");
    // The lock still holds for everything else of the hub's.
    let dev = hub.device_by_serial(serial).unwrap();
    assert_eq!(dev.router.lock_holder(Instant::now()), Some(holder.id()));

    // More of the storm: no second mute.
    for n in 0..12 {
        hub.bus().publish(clip(serial, n));
    }
    let viewer = rig.session(Role::Viewer, "Viewer");
    let list = rig.ask(
        &viewer,
        json!({"t": "alarm.list", "id": 2, "state": "active"}),
    );
    let alarms = list["alarms"].as_array().unwrap();
    assert_eq!(alarms.len(), 1, "{list}");
    assert_eq!(alarms[0]["code"], "clip");
    assert!(
        alarms[0]["message"]
            .as_str()
            .unwrap()
            .contains("the hub muted")
    );
    assert_eq!(wire.user_mutes().len(), 1);

    // The history names the rule.
    rig.until("the mute's record", || {
        let ev = rig.ask(
            &viewer,
            json!({"t": "event.list", "id": 3, "serial": serial}),
        );
        ev["records"].as_array().is_some_and(|r| {
            r.iter().any(|r| {
                r["by"] == MUTE_BY
                    && r["source"] == MUTE_SOURCE
                    && r["session"] == u32::from(INTERNAL_SESSION_ID)
            })
        })
    });
    // And the alarm itself, raised.
    let ev = rig.ask(
        &viewer,
        json!({"t": "event.list", "id": 4, "serial": serial}),
    );
    assert!(
        ev["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "alarm" && r["source"] == "clip"),
        "{ev}"
    );

    // The storm passes: cleared, nothing unmuted. A new storm mutes once more.
    rig.clock.advance(Duration::from_secs(61));
    rig.until("calm begins", || {
        rig.ask(
            &viewer,
            json!({"t": "alarm.list", "id": 5, "state": "active"}),
        )["alarms"]
            .as_array()
            .is_some()
    });
    rig.clock.advance(Duration::from_secs(61));
    rig.until("the storm's end", || {
        rig.ask(
            &viewer,
            json!({"t": "alarm.list", "id": 6, "state": "active"}),
        )["alarms"]
            == json!([])
    });
    assert_eq!(wire.user_mutes(), vec![vec![1]], "never an unmute");
    for n in 0..12 {
        hub.bus().publish(clip(serial, n));
    }
    rig.until("the second mute", || wire.user_mutes().len() == 2);
    assert_eq!(wire.user_mutes(), vec![vec![1], vec![1]]);
}

#[test]
fn wp_p2al_rules_are_an_admins_and_kept_in_a_private_file() {
    let rig = Rig::new("rules");
    let viewer = rig.session(Role::Viewer, "v");
    let control = rig.session(Role::Control, "c");
    let admin = rig.session(Role::Admin, "a");
    let get = rig.ask(&viewer, json!({"t": "alarm.rules.get", "id": 1}));
    assert_eq!(get["t"], "ok", "a viewer reads them: {get}");
    assert_eq!(get["v"], 1);
    assert_eq!(get["rules"], Rules::default().to_json());
    let change =
        json!({"t": "alarm.rules.set", "id": 2, "rules": {"temperature": {"above_c": 65}}});
    for s in [&viewer, &control] {
        let r = rig.ask(s, change.clone());
        assert_eq!(r["code"], "denied", "{r}");
    }
    assert!(!rig.dir.join(FILE_NAME).exists());
    let bad = rig.ask(
        &admin,
        json!({"t": "alarm.rules.set", "id": 3, "rules": {"temperature": {"above_c": "hot"}}}),
    );
    assert_eq!(bad["code"], "bad_request", "{bad}");
    let ok = rig.ask(&admin, change);
    assert_eq!(ok["t"], "ok", "{ok}");
    assert_eq!(ok["rules"]["temperature"]["above_c"], 65.0);
    let file = rig.dir.join(FILE_NAME);
    assert_eq!(load(&file, T0).num("temperature", "above_c"), 65.0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // A viewer may list, not acknowledge.
    let ack = rig.ask(
        &viewer,
        json!({"t": "alarm.ack", "id": 4, "alarm": "device:1"}),
    );
    assert_eq!(ack["code"], "denied");
    let none = rig.ask(
        &control,
        json!({"t": "alarm.ack", "id": 5, "alarm": "device:1"}),
    );
    assert_eq!(none["code"], "not_found");
    let bad = rig.ask(&control, json!({"t": "alarm.list", "id": 6, "state": "on"}));
    assert_eq!(bad["code"], "bad_request");
    // Not this family's: no answer.
    assert!(
        rig.hub
            .handle()
            .service_message(&admin, &json!({"t": "alarm.other", "id": 7}))
            .is_none()
    );
}

#[test]
fn wp_p2al_audio_link_alarms_are_listed_and_acknowledged_beside_the_devices() {
    let rig = Rig::new("alink");
    let hub = rig.hub.handle();
    let control = rig.session(Role::Control, "Front of house");
    let watch = hub.bus().subscribe("test", 64);
    // An audio link alarm, as the system's poll raises it.
    let raised = hub.inner.system.system_mute_seen("Sender", true);
    assert_eq!(raised.len(), 1);
    // And a device alarm.
    hub.bus().publish(health("P2AL-HOT", None, 0, Some(80.0)));
    rig.until("both alarms", || {
        rig.ask(&control, json!({"t": "alarm.list", "id": 1}))["alarms"]
            .as_array()
            .is_some_and(|a| a.len() == 2)
    });
    let list = rig.ask(&control, json!({"t": "alarm.list", "id": 2}));
    let sources: Vec<&str> = list["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["source"].as_str().unwrap())
        .collect();
    assert!(
        sources.contains(&"alink") && sources.contains(&"device"),
        "{list}"
    );
    let alink = list["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["source"] == "alink")
        .unwrap()
        .clone();
    assert_eq!(alink["code"], "system_muted");
    assert!(alink["id"].as_str().unwrap().starts_with("alink:"));
    // By serial: the device's alone.
    let one = rig.ask(
        &control,
        json!({"t": "alarm.list", "id": 3, "serial": "P2AL-HOT"}),
    );
    assert_eq!(one["alarms"].as_array().unwrap().len(), 1, "{one}");
    assert_eq!(one["alarms"][0]["code"], "temperature");
    // Acknowledge each.
    let a = rig.ask(
        &control,
        json!({"t": "alarm.ack", "id": 4, "alarm": alink["id"]}),
    );
    assert_eq!(a["alarm"]["state"], "acknowledged", "{a}");
    assert_eq!(a["alarm"]["acknowledged_by"], "Front of house");
    let d = rig.ask(
        &control,
        json!({"t": "alarm.ack", "id": 5, "alarm": one["alarms"][0]["id"]}),
    );
    assert_eq!(d["alarm"]["state"], "acknowledged", "{d}");
    assert_eq!(d["alarm"]["acknowledged_by"], "Front of house");
    // `system.alarms` keeps its own view of the audio link's.
    assert_eq!(hub.system_alarms()[0].state, AlarmState::Acknowledged);
    // Each change went onto the bus, the audio link's too.
    let seen: Vec<(String, AlarmState)> = watch
        .drain()
        .iter()
        .filter_map(|m| match &**m {
            BusMessage::Alarm { source, state, .. } => Some((source.clone(), *state)),
            _ => None,
        })
        .collect();
    for want in [
        ("alink".to_owned(), AlarmState::Raised),
        ("device".to_owned(), AlarmState::Raised),
        ("alink".to_owned(), AlarmState::Acknowledged),
        ("device".to_owned(), AlarmState::Acknowledged),
    ] {
        assert!(seen.contains(&want), "{want:?} in {seen:?}");
    }
}
