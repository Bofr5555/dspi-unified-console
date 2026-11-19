//! WP ATDEMO: the demo hub's simulated room fails closed. The room hears
//! only the demo units it was installed with, as the same attachments: a
//! hub without a room offers no simulated microphone and refuses
//! `system.measure.demo_mic`; a unit outside the room (standing in for a
//! real DSPi) is refused by name, before a session and during one; a demo
//! serial attached again later (another device) is refused too; and the
//! core then starts nothing. The whole measurement through the room is
//! `uconsole`'s test (apps/terminal/src/hub.rs, `tests_wp_atdemo`).
#![cfg(feature = "demo-room")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dspi_core::{
    DspiCore, DspiError, MeasureOutput, MeasureRole, MeasureSystemSetup, MeasureSystemUnit,
};
use dspi_hub::system::measure::DemoRoomUnit;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_measure::system::messages::demo;
use dspi_transport::FakeDevice;
use dspi_transport::fake::{AlinkSetup, FakeTimeBase, FakeTimeBaseUnit};
use serde_json::{Value, json};

fn store_dir(name: &str) -> PathBuf {
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir()
        .join("wp-atdemo")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn hub(name: &str) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir(name))
        }),
        initial_scan: false,
        ..HubConfig::in_process("wp-atdemo hub")
    })
    .unwrap();
    hub.handle().set_system_measure_settle(Duration::ZERO);
    hub
}

/// `serials` on one line time base, attached; their remotes.
fn units(
    hub: &Hub,
    tb: &FakeTimeBase,
    serials: &[&str],
) -> Vec<(String, u8, dspi_transport::FakeRemote)> {
    serials
        .iter()
        .map(|s| {
            let d =
                FakeDevice::demo_on_time_base(tb, FakeTimeBaseUnit::Line(AlinkSetup::default()))
                    .with_serial(s);
            let r = d.remote();
            let h = hub.attach(Box::new(d), 0).unwrap();
            ((*s).to_owned(), h, r)
        })
        .collect()
}

fn connect(hub: &Hub, serial: &str) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-atdemo".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let d = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == serial)
        .unwrap();
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

fn ask(core: &DspiCore, body: Value) -> Result<Value, DspiError> {
    let Value::Object(m) = body else {
        panic!("an object")
    };
    core.link_hub_request(demo::T, m)
}

fn refused_naming(r: Result<Value, DspiError>, serial: &str) {
    match r {
        Err(DspiError::Link { code, msg }) => {
            assert_eq!(code, "unsupported", "{msg}");
            assert!(msg.contains(serial), "{msg}");
        }
        other => panic!("not refused: {other:?}"),
    }
}

fn setup(serials: &[&str]) -> MeasureSystemSetup {
    MeasureSystemSetup {
        units: serials
            .iter()
            .map(|s| MeasureSystemUnit {
                serial: (*s).into(),
                outputs: vec![MeasureOutput {
                    output: 0,
                    role: MeasureRole::Full,
                    speaker: format!("{s}/0"),
                    ..MeasureOutput::default()
                }],
            })
            .collect(),
        passes: 3,
        sweep_ms: 1_000,
        gap_ms: 300,
        capture_rate_hz: 48_000,
        ..MeasureSystemSetup::default()
    }
}

/// A hub without the room: no cap, the message refused, the core's
/// simulated microphone refused.
#[test]
fn wp_atdemo_a_hub_without_the_room_offers_no_simulated_microphone() {
    let hub = hub("none");
    let tb = FakeTimeBase::line(0x31);
    units(&hub, &tb, &["AD-1"]);
    let core = connect(&hub, "AD-1");
    let info = core.measure_system_sim_info();
    assert!(!info.available && !info.on, "{info:?}");
    assert!(matches!(
        core.measure_system_sim_set(true),
        Err(DspiError::Link { ref code, .. }) if code == "unsupported"
    ));
    let r = ask(&core, json!({"units": ["AD-1"]}));
    assert!(
        matches!(r, Err(DspiError::Link { ref code, .. }) if code == "unsupported"),
        "{r:?}"
    );
}

/// The room hears its own units only: another unit (a real DSPi's stand-in)
/// is refused by name before a session (the core starts nothing) and
/// during one (nothing heard); a demo serial attached again is refused.
#[test]
fn wp_atdemo_the_room_hears_only_its_own_demo_attachments() {
    let hub = hub("own");
    let tb = FakeTimeBase::line(0x32);
    let mut attached = units(&hub, &tb, &["AD-A", "AD-B", "REAL-1"]);
    let room: Vec<DemoRoomUnit> = attached
        .iter()
        .filter(|(s, _, _)| s.starts_with("AD-"))
        .map(|(s, _, r)| DemoRoomUnit {
            serial: s.clone(),
            remote: r.clone(),
            acoustic_ms: vec![3.0],
        })
        .collect();
    hub.handle().set_system_measure_demo_room(room, 1).unwrap();
    assert!(
        hub.handle()
            .set_system_measure_demo_room(
                vec![DemoRoomUnit {
                    serial: "NOT-HERE".into(),
                    remote: attached[0].2.clone(),
                    acoustic_ms: Vec::new(),
                }],
                1
            )
            .is_err(),
        "a unit must be attached"
    );
    let core = connect(&hub, "AD-A");
    core.set_user_volume(-20.0).unwrap();
    // Offered here, and heard for the room's units.
    assert!(core.measure_system_sim_info().available);
    let ok = ask(&core, json!({"units": ["AD-A", "AD-B"]})).unwrap();
    assert_eq!(ok["ok"], true, "{ok}");
    refused_naming(ask(&core, json!({"units": ["AD-A", "REAL-1"]})), "REAL-1");

    // The core with the simulated microphone on: a group with the other
    // unit does not start, and no session is opened.
    assert!(core.measure_system_sim_set(true).unwrap().on);
    let s = setup(&["AD-A", "REAL-1"]);
    let real = connect(&hub, "REAL-1");
    real.set_user_volume(-20.0).unwrap();
    let checks = vec![
        core.measure_system_unit_check(s.clone(), s.units[0].clone())
            .unwrap(),
        real.measure_system_unit_check(s.clone(), s.units[1].clone())
            .unwrap(),
    ];
    let r = core.measure_system_start(s, checks);
    match r {
        Err(DspiError::Link { code, msg }) => {
            assert_eq!(code, "unsupported");
            assert!(msg.contains("REAL-1"), "{msg}");
        }
        other => panic!("started: {other:?}"),
    }
    assert_eq!(hub.handle().system_measure_sessions(), 0, "nothing opened");
    assert!(core.measure_system_sim_info().issue.is_some());

    // A session over both opened by hand: its capture is refused, by name.
    let open = core
        .link_hub_request(
            "system.measure.open",
            json!({"units": ["AD-A", "REAL-1"]})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let session = open["session"].as_str().unwrap().to_owned();
    refused_naming(ask(&core, json!({"session": session, "from": 0})), "REAL-1");
    let _ = core.link_hub_request(
        "system.measure.close",
        json!({"session": session}).as_object().unwrap().clone(),
    );

    // A demo serial attached again (another device under it): refused.
    let (_, h, _) = attached.remove(1);
    hub.handle().detach(h).unwrap();
    refused_naming(ask(&core, json!({"units": ["AD-B"]})), "AD-B");
    let again = FakeDevice::demo_on_time_base(&tb, FakeTimeBaseUnit::Line(AlinkSetup::default()))
        .with_serial("AD-B");
    hub.attach(Box::new(again), 0).unwrap();
    refused_naming(ask(&core, json!({"units": ["AD-B"]})), "AD-B");
    assert_eq!(ask(&core, json!({"units": ["AD-A"]})).unwrap()["ok"], true);
}
