//! WP P2CC2: the core's typed calls for the hub's admin families
//! (`dspi_core::hub_server::admin`) against the real hub: alarms and their
//! rules, schedules with `schedule.ran`, a real `schedule_failed` alarm with
//! `alarm.changed` and its acknowledgement, notifications out with a secret
//! shown once, the MQTT setting, and API keys. The client is the real one
//! (`DspiCore` over DSPi Link), paired with the hub's PIN (Control) and given
//! Viewer or Admin on the hub's machine where a test needs it. The wire shapes
//! against the fake hub are in `crates/dspi-core/src/hub_server/tests_wp_p2cc2.rs`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::hub_server::ALARM_CODES;
use dspi_core::{
    DspiCore, DspiError, HubAlarmRule, HubOutboundTarget, HubScheduleAction, HubScheduleRule,
    HubServerEvent, HubServerListener,
};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role, ScheduleConfig, ServiceConfig};
use dspi_transport::{Capture, FakeDevice, FakeOptions};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "P2CC2-1";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2cc2").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A hub with alarms and schedules on (outbound, MQTT and API keys are always
/// there), and one device.
fn hub_in(dir: &std::path::Path) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        services: ServiceConfig {
            alarms: true,
            schedule: Some(ScheduleConfig {
                zone: Some("UTC".into()),
                not_before_s: Some(0),
            }),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2CC2 hub")
    })
    .unwrap();
    let device = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(SERIAL);
    hub.attach(Box::new(device), 0).unwrap();
    hub
}

/// A core paired on `hub` with the hub's PIN (Control), given `role` on the
/// hub's machine, bound to the device.
fn client(hub: &Hub, role: Role) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-p2cc2".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    if role != Role::Control {
        let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
        assert!(net.set_role(cid, role));
    }
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core
}

#[derive(Default)]
struct Heard(Mutex<Vec<HubServerEvent>>);

impl HubServerListener for Heard {
    fn on_hub_server_event(&self, event: HubServerEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Heard {
    fn wait_for(&self, what: &str, f: impl Fn(&HubServerEvent) -> bool) -> HubServerEvent {
        let mut found = None;
        wait_until(what, || {
            found = self.0.lock().unwrap().iter().find(|e| f(e)).cloned();
            found.is_some()
        });
        found.unwrap()
    }
}

#[test]
fn wp_p2cc2_alarms_and_schedules_on_the_real_hub() {
    let dir = scratch("alarms");
    let hub = hub_in(&dir);
    let admin = client(&hub, Role::Admin);
    let caps = admin.hub_caps();
    assert!(
        caps.alarms && caps.schedule && caps.outbound && caps.mqtt && caps.api_keys,
        "{caps:?}"
    );
    assert!(!caps.rest, "{caps:?}");

    // Every device code the hub raises has a rule here and a notice row.
    let rules = admin.hub_alarm_rules_get().unwrap();
    for code in dspi_hub::device_alarms::CODES {
        assert!(
            ALARM_CODES.contains(&code),
            "{code}: not in the core's list"
        );
        assert!(rules.iter().any(|r| r.code == code), "{code}: no rule");
        let row = dspi_core::notice_for_key(format!("alarm.{code}"), String::new());
        assert_eq!(row.key, format!("alarm.{code}"), "{code}: no notice row");
    }
    let cpu = rules.iter().find(|r| r.code == "cpu").unwrap();
    assert_eq!((cpu.above_pct, cpu.for_s), (Some(90), Some(10)));
    let changed = admin
        .hub_alarm_rules_set(vec![HubAlarmRule {
            code: "cpu".into(),
            on: true,
            above_pct: Some(80),
            ..HubAlarmRule::default()
        }])
        .unwrap();
    let cpu = changed.iter().find(|r| r.code == "cpu").unwrap();
    assert_eq!(
        (cpu.above_pct, cpu.for_s),
        (Some(80), Some(10)),
        "the rest kept"
    );
    let e = admin
        .hub_alarm_rules_set(vec![HubAlarmRule {
            code: "cpu".into(),
            on: true,
            above_pct: Some(101),
            ..HubAlarmRule::default()
        }])
        .unwrap_err();
    assert!(
        matches!(&e, DspiError::InvalidArgument { msg } if msg.contains("above_pct")),
        "{e:?}"
    );

    // A rule: the device's mute and a device the hub has never seen.
    let control = client(&hub, Role::Control);
    let heard = Arc::new(Heard::default());
    control.set_hub_server_listener(Some(heard.clone()));
    let rule = HubScheduleRule {
        name: "Quiet".into(),
        enabled: false,
        days: vec!["mon".into()],
        time: "03:00".into(),
        actions: vec![HubScheduleAction {
            t: "param.set".into(),
            path: "vol.mute".into(),
            flag: Some(false),
            ..HubScheduleAction::default()
        }],
        targets: vec![SERIAL.into(), "NO-SUCH-DSPI".into()],
        ..HubScheduleRule::default()
    };
    // Schedules are an Admin's to change; every role reads them.
    assert_eq!(
        control.hub_schedule_set(rule.clone()).unwrap_err(),
        DspiError::Denied
    );
    let made = admin.hub_schedule_set(rule).unwrap();
    assert!(!made.id.is_empty(), "{made:?}");
    assert_eq!(made.actions[0].flag, Some(false));
    let listed = control.hub_schedules().unwrap();
    assert_eq!(listed.zone, "UTC");
    assert!(!listed.clock_unset, "{listed:?}");
    assert!(
        listed
            .rules
            .iter()
            .any(|r| r.id == made.id && r.name == "Quiet")
    );

    // Run now: the device's row goes through, the unknown one fails; the run
    // is announced and a real `schedule_failed` alarm is raised.
    let run = admin.hub_schedule_run_now(made.id.clone()).unwrap();
    assert_eq!(run.rule, made.id);
    assert_eq!(run.by, "wp-p2cc2");
    assert_eq!(run.results.len(), 2, "{run:?}");
    assert!(run.results[0].ok, "{run:?}");
    assert_eq!(run.results[1].code, "not_found", "{run:?}");
    let ran = heard.wait_for("schedule.ran", |e| e.kind == "schedule.ran");
    assert_eq!(ran.run.unwrap().rule, made.id);
    let raised = heard.wait_for("alarm.changed", |e| {
        e.kind == "alarm.changed"
            && e.alarm
                .as_ref()
                .is_some_and(|a| a.code == "schedule_failed")
    });
    let alarm = raised.alarm.unwrap();
    assert_eq!(alarm.subject, made.id);
    assert_eq!(alarm.notice_key, "alarm.schedule_failed");
    let listed = control.hub_alarms(String::new(), "active".into()).unwrap();
    assert!(listed.iter().any(|a| a.id == alarm.id), "{listed:?}");
    let n = dspi_core::hub_alarm_notice(alarm.clone(), "18:30".into());
    assert_eq!(n.detail, alarm.message);
    assert!(!n.reason.contains('{'), "{n:?}");

    // A Viewer reads alarms but does not acknowledge; Control does.
    let viewer = client(&hub, Role::Viewer);
    assert!(
        !viewer
            .hub_alarms(String::new(), String::new())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        viewer.hub_alarm_ack(alarm.id.clone()).unwrap_err(),
        DspiError::Denied
    );
    let acked = control.hub_alarm_ack(alarm.id.clone()).unwrap();
    assert_eq!(acked.id, alarm.id);
    assert_ne!(acked.state, "raised", "{acked:?}");
    assert_eq!(acked.acknowledged_by, "wp-p2cc2");
    let e = control.hub_alarm_ack("device:999999".into()).unwrap_err();
    assert_eq!(e.notice_key(), "dlink.not_found", "{e:?}");

    admin.hub_schedule_delete(made.id.clone()).unwrap();
    let e = admin.hub_schedule_delete(made.id).unwrap_err();
    assert_eq!(e.notice_key(), "dlink.not_found", "{e:?}");
    for c in [&admin, &control, &viewer] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p2cc2_outbound_mqtt_and_api_keys_on_the_real_hub() {
    let dir = scratch("settings");
    let hub = hub_in(&dir);
    let admin = client(&hub, Role::Admin);
    let control = client(&hub, Role::Control);

    // Notifications out: Admin only; a new secret is shown once.
    assert_eq!(control.hub_outbound_get().unwrap_err(), DspiError::Denied);
    assert!(admin.hub_outbound_get().unwrap().is_empty());
    let set = admin
        .hub_outbound_set(vec![HubOutboundTarget {
            name: "Bench".into(),
            kind: "webhook".into(),
            url: "http://127.0.0.1:9/hook".into(),
            enabled: true,
            kinds: vec!["alarm.raised".into()],
            secret: "new".into(),
            plaintext: true,
            ..HubOutboundTarget::default()
        }])
        .unwrap();
    assert_eq!(set.targets.len(), 1);
    let id = set.targets[0].id.clone();
    assert_eq!(set.secrets.len(), 1, "{set:?}");
    assert_eq!(set.secrets[0].target, id);
    let secret = set.secrets[0].secret.reveal();
    assert_eq!(secret.len(), 64);
    assert!(
        !format!("{set:?}").contains(&secret),
        "Debug shows no secret"
    );
    let got = admin.hub_outbound_get().unwrap();
    assert_eq!(got[0].secret, "set");
    assert!(!got[0].encrypted);
    // Sent back as it is: the secret kept, none made.
    let again = admin.hub_outbound_set(got).unwrap();
    assert!(again.secrets.is_empty());
    assert_eq!(again.targets[0].secret, "set");
    let e = admin
        .hub_outbound_set(vec![HubOutboundTarget {
            kind: "webhook".into(),
            url: "http://8.8.8.8/hook".into(),
            plaintext: true,
            ..HubOutboundTarget::default()
        }])
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let test = admin.hub_outbound_test(id).unwrap();
    assert!(!test.delivered, "{test:?}");
    assert_eq!(
        admin
            .hub_outbound_test("nope".into())
            .unwrap_err()
            .notice_key(),
        "dlink.not_found"
    );

    // MQTT: off by default; the password goes in, comes back as `set`.
    assert_eq!(control.hub_mqtt_get().unwrap_err(), DspiError::Denied);
    let mut m = admin.hub_mqtt_get().unwrap();
    assert!(!m.enabled);
    assert_eq!(m.password, "not set");
    m.broker = "127.0.0.1".into();
    m.username = "hub".into();
    m.password = "broker-pass".into();
    let m = admin.hub_mqtt_set(m).unwrap();
    assert_eq!(
        (m.broker.as_str(), m.password.as_str()),
        ("127.0.0.1", "set")
    );
    assert!(!format!("{m:?}").contains("broker-pass"));
    // Sent back as it is: the password kept.
    let m = admin.hub_mqtt_set(m).unwrap();
    assert_eq!(m.password, "set");
    let st = admin.hub_mqtt_status().unwrap();
    assert_eq!(st.state, "off", "{st:?}");
    let mut bad = m.clone();
    bad.password = "p".into();
    bad.username = String::new();
    assert!(matches!(
        admin.hub_mqtt_set(bad),
        Err(DspiError::InvalidArgument { .. })
    ));

    // API keys: minted once, listed without the consoles, revoked.
    assert_eq!(
        control
            .hub_api_key_mint("Script".into(), "viewer".into(), false)
            .unwrap_err(),
        DspiError::Denied
    );
    let minted = admin
        .hub_api_key_mint("Script".into(), "viewer".into(), false)
        .unwrap();
    assert_eq!(minted.key.reveal().len(), 43);
    assert_eq!(
        (minted.name.as_str(), minted.role.as_str()),
        ("Script", "viewer")
    );
    assert!(!format!("{minted:?}").contains(&minted.key.reveal()));
    let keys = admin.hub_api_keys().unwrap();
    assert_eq!(keys.len(), 1, "only the key, not the consoles: {keys:?}");
    assert_eq!(keys[0].cid, minted.cid);
    // A console's pairing is not a key: refused here, nothing revoked.
    let console = hub
        .network()
        .unwrap()
        .auth()
        .clients()
        .iter()
        .map(|c| c.cid)
        .find(|c| *c != minted.cid)
        .unwrap();
    assert!(matches!(
        admin.hub_api_key_revoke(console),
        Err(DspiError::InvalidArgument { .. })
    ));
    // Read-only mode: the write refused, nothing changed.
    admin.set_client_read_only(true).unwrap();
    assert_eq!(
        admin.hub_api_key_revoke(minted.cid).unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(
        admin
            .hub_api_key_mint("Two".into(), "viewer".into(), false)
            .unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(admin.hub_api_keys().unwrap().len(), 1);
    admin.set_client_read_only(false).unwrap();
    admin.hub_api_key_revoke(minted.cid).unwrap();
    assert!(admin.hub_api_keys().unwrap().is_empty());
    assert_eq!(
        hub.network().unwrap().auth().clients().len(),
        2,
        "the consoles stay"
    );
    for c in [&admin, &control] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
