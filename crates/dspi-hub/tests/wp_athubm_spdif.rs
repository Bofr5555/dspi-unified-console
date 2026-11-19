//! WP ATHUBM: S/PDIF units in the hub's system model, on a real hub with
//! WP ATFAKE's units on one S/PDIF time base:
//!
//! - the alignment writes `EXT_ALINK_SPDIF` on the source (mode 2, its slots)
//!   and on each follower (mode 1, a D inside its limits, its lock-error
//!   action), as it writes `EXT_ALINK_USB` on USB-direct units; a unit that
//!   was a source and becomes a follower has its `tx_mask` zeroed (review
//!   atab carry-over), and a second alignment writes nothing;
//! - `system.status` reads `EXT_TIME_BASE` on every unit;
//! - an 0xE4 (a follower's holdover and resume) raises and clears
//!   `spdif_lost`; a follower in lock error raises `spdif_lock_error` at a read.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::system::alarms::{AlarmState, SPDIF_LOCK_ERROR, SPDIF_LOST};
use dspi_hub::system::{SystemStatus, WriteResult};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_transport::FakeDevice;
use dspi_transport::fake::{
    FakeRemote, FakeTimeBase, FakeTimeBaseUnit, SPDIF_D_MIN, SpdifSetup, TimeBaseFault,
};
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-athubm").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// The source SRC (stamping every slot until the hub sets slot 0), the
/// follower F1 (D 192), and F2, which runs as a second source with slot 0
/// stamped until the model makes it a follower.
fn hub() -> (Hub, FakeTimeBase, Vec<FakeRemote>) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("ATHUBM hub")
    })
    .unwrap();
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));
    let tb = FakeTimeBase::spdif(0x3742);
    let mut remotes = Vec::new();
    for (serial, setup) in [
        ("HW-SRC", SpdifSetup::source()),
        ("HW-F1", SpdifSetup::follower(192)),
        (
            "HW-F2",
            SpdifSetup {
                tx_mask: 1,
                ..SpdifSetup::source()
            },
        ),
    ] {
        let dev =
            FakeDevice::demo_on_time_base(&tb, FakeTimeBaseUnit::Spdif(setup)).with_serial(serial);
        remotes.push(dev.remote());
        hub.attach(Box::new(dev), 0).unwrap();
    }
    let model = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "units": [
            {"id": "SRC", "name": "Front", "serial": "HW-SRC", "room": "main", "transport": "spdif",
             "spdifSource": true, "spdifTxMask": 1,
             "speakers": [{"output": 2, "distanceM": 3.0}]},
            {"id": "F1", "name": "Rear", "serial": "HW-F1", "room": "main", "transport": "spdif",
             "upstream": "SRC", "speakers": [{"output": 0, "distanceM": 3.0}]},
            {"id": "F2", "name": "Height", "serial": "HW-F2", "room": "main", "transport": "spdif",
             "upstream": "SRC", "onLockError": "play_unaligned",
             "speakers": [{"output": 0, "distanceM": 3.0}]}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    (hub, tb, remotes)
}

fn spdif_writes(w: &[WriteResult]) -> Vec<(String, String)> {
    w.iter()
        .filter(|w| w.what == "spdif")
        .map(|w| (w.element.clone(), w.result.clone()))
        .collect()
}

fn on(s: &SystemStatus, code: &str) -> bool {
    s.alarms
        .iter()
        .any(|a| a.code == code && a.subject == "F1" && a.state != AlarmState::Cleared)
}

#[test]
fn wp_athubm_the_alignment_writes_the_spdif_roles() {
    let (hub, _tb, _remotes) = hub();
    let h = hub.handle();
    let r = h.system_align(true);
    assert!(r.applied);
    assert_eq!(
        spdif_writes(&r.writes),
        [
            ("SRC".to_owned(), "ok".to_owned()),
            ("F1".to_owned(), "ok".to_owned()),
            ("F2".to_owned(), "ok".to_owned()),
        ],
        "{:#?}",
        r.writes
    );
    let dom = &r.alignment.spdif_domains[0];
    assert_eq!(
        (dom.source.as_str(), dom.d, dom.d_auto),
        ("SRC", SPDIF_D_MIN, true)
    );

    let s = h.system_status();
    let seen = |serial: &str| s.observed[serial].spdif.clone().unwrap();
    let src = seen("HW-SRC");
    assert_eq!((src.mode.as_str(), src.tx_mask), ("source", 1));
    let f1 = seen("HW-F1");
    assert_eq!(
        (f1.mode.as_str(), f1.d, f1.tx_mask),
        ("follower", SPDIF_D_MIN, 0)
    );
    // A source turned follower: its mask zeroed, its D and action written.
    let f2 = seen("HW-F2");
    assert_eq!(
        (
            f2.mode.as_str(),
            f2.d,
            f2.tx_mask,
            f2.on_lock_error.as_str()
        ),
        ("follower", SPDIF_D_MIN, 0, "play_unaligned")
    );
    // The source's speakers wait for its followers: D samples at 48 kHz.
    let p = s.alignment.units.iter().find(|p| p.unit == "SRC").unwrap();
    assert!(
        (p.out_delays[0].1 - f32::from(SPDIF_D_MIN) / 48.0).abs() < 0.001,
        "{:?}",
        p.out_delays
    );
    // EXT_TIME_BASE on every unit.
    let tb = s.observed["HW-F1"].time_base.clone().unwrap();
    assert_eq!(
        (
            tb.entries[0].transport.as_str(),
            tb.entries[0].role.as_str()
        ),
        ("spdif", "follower")
    );
    let tb = s.observed["HW-SRC"].time_base.clone().unwrap();
    assert_eq!(tb.entries[0].role, "source");
    assert!(s.observed["HW-F1"].time_base_events);

    // Again: nothing to write, nothing re-anchors.
    let r = h.system_align(true);
    assert!(
        spdif_writes(&r.writes)
            .iter()
            .all(|(_, x)| x == "unchanged"),
        "{:#?}",
        r.writes
    );
    let delays: Vec<&WriteResult> = r
        .writes
        .iter()
        .filter(|w| w.what.starts_with("out_delay"))
        .collect();
    assert!(
        delays.iter().all(|w| w.result == "unchanged"),
        "{delays:#?}"
    );
}

#[test]
fn wp_athubm_spdif_alarms_from_events_and_reads() {
    let (hub, _tb, remotes) = hub();
    let h = hub.handle();
    h.system_align(true);
    // Settle the followers' anchoring, then learn the 0xE4 feature bit.
    std::thread::sleep(Duration::from_millis(60));
    let s = h.system_status();
    assert!(!on(&s, SPDIF_LOST), "{:?}", s.alarms);
    let f1 = &remotes[1];
    assert!(f1.time_base_fault(TimeBaseFault::Holdover));
    let start = Instant::now();
    loop {
        let a = h.system_alarms();
        if let Some(a) = a
            .iter()
            .find(|a| a.code == SPDIF_LOST && a.state != AlarmState::Cleared)
        {
            assert_eq!(a.subject, "F1");
            assert_eq!(a.severity, "critical");
            break;
        }
        assert!(start.elapsed() < WAIT, "no alarm from the 0xE4: {a:?}");
        // The fake works its time base out at the next request.
        let _ = f1.time_base_t();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(f1.time_base_fault(TimeBaseFault::Resume));
    let start = Instant::now();
    while h
        .system_alarms()
        .iter()
        .any(|a| a.code == SPDIF_LOST && a.state != AlarmState::Cleared)
    {
        assert!(start.elapsed() < WAIT, "not cleared by the resume");
        let _ = f1.time_base_t();
        std::thread::sleep(Duration::from_millis(20));
    }
    // Lock error, seen at a read.
    assert!(f1.time_base_fault(TimeBaseFault::LockError(true)));
    let s = h.system_status();
    assert!(on(&s, SPDIF_LOCK_ERROR), "{:?}", s.alarms);
    assert!(
        s.alignment
            .issues
            .iter()
            .any(|i| i.code == "spdif_lock_error" && i.subject.as_deref() == Some("F1"))
    );
    assert!(f1.time_base_fault(TimeBaseFault::LockError(false)));
    let s = h.system_status();
    assert!(!on(&s, SPDIF_LOCK_ERROR), "{:?}", s.alarms);
}
