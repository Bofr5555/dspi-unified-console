//! WP P2FFIX (review p2f F1): the core's adopted hub guard (a guarded
//! `snapshot.recall` on the hub, WP P2CC3) on a device that is not the open
//! one, the open device changing mid-guard, the session dropping, and a
//! guarded recall from a session with no bound device, against the real hub.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, GuardPhase};
use dspi_hub::{AccessMode, HistoryConfig, Hub, HubConfig, NetworkConfig, ServiceConfig};
use dspi_transport::{Capture, FakeDevice, FakeOptions};

const WAIT: Duration = Duration::from_secs(20);
const OPEN: &str = "P2FFIX-1";
const OTHER: &str = "P2FFIX-2";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-p2ffix").join(format!(
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

fn device(serial: &str) -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(serial)
}

fn hub_in(dir: &std::path::Path) -> (Hub, u8, u8) {
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
            inventory: true,
            identify: true,
            history: Some(HistoryConfig::default()),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2FFIX hub")
    })
    .unwrap();
    let open = hub.attach(Box::new(device(OPEN)), 0).unwrap();
    let other = hub.attach(Box::new(device(OTHER)), 0).unwrap();
    (hub, open, other)
}

/// Paired (Control), bound to `bind` if given.
fn client(hub: &Hub, bind: Option<&str>) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-p2ffix".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    wait_until("the device list", || core.link_devices().len() == 2);
    if let Some(serial) = bind {
        let d = core
            .link_devices()
            .into_iter()
            .find(|d| d.serial == serial)
            .unwrap();
        core.link_select_device(d.handle, d.serial).unwrap();
    }
    core
}

fn preamp(core: &DspiCore) -> f32 {
    core.fetch_all().unwrap();
    core.get_state().preamp_db
}

fn hub_active(core: &DspiCore, serial: &str) -> serde_json::Value {
    let handle = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == serial)
        .unwrap()
        .handle;
    let mut f = serde_json::Map::new();
    f.insert("handle".into(), handle.into());
    core.link_hub_request("apply.status", f).unwrap()["active"].clone()
}

/// A snapshot of `observer`'s open device at -2 dB, saved to the hub; the device then at -6.
fn seed(observer: &DspiCore, dir: &std::path::Path, name: &str) -> String {
    observer
        .snapshot_set_dir(dir.join(name).to_string_lossy().into_owned())
        .unwrap();
    observer.fetch_all().unwrap();
    observer.set_preamp(-2.0).unwrap();
    let snap = observer
        .snapshot_capture("At -2".into(), String::new())
        .unwrap();
    observer.hub_snapshots_save(snap.id.clone()).unwrap();
    observer.set_preamp(-6.0).unwrap();
    assert_eq!(preamp(observer), -6.0);
    snap.id
}

#[test]
fn wp_p2ffix_an_adopted_guard_on_a_device_that_is_not_the_open_one() {
    let dir = scratch("other");
    let (hub, _, _) = hub_in(&dir);
    let control = client(&hub, Some(OPEN));
    let observer = client(&hub, Some(OTHER));
    let id = seed(&observer, &dir, "obs");

    // Revert on OTHER from a core bound to OPEN.
    let r = control
        .hub_snapshots_recall(OTHER.into(), id.clone(), true, 30_000)
        .unwrap();
    assert!(!r.guard.is_empty(), "{r:?}");
    assert_eq!(r.serial, OTHER);
    let st = control.apply_status();
    assert_eq!(st.phase, GuardPhase::Waiting, "{st:?}");
    assert!(st.hub);
    assert_eq!(preamp(&observer), -2.0, "OTHER recalled");
    assert!(
        !hub_active(&control, OTHER).is_null(),
        "the hub's guard runs on OTHER"
    );
    assert!(hub_active(&control, OPEN).is_null(), "none on OPEN");
    let end = control.apply_revert().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "reverted"),
        "{end:?}"
    );
    assert_eq!(preamp(&observer), -6.0, "OTHER put back by the hub");
    assert!(hub_active(&control, OTHER).is_null());

    // Keep on OTHER.
    control
        .hub_snapshots_recall(OTHER.into(), id.clone(), true, 0)
        .unwrap();
    assert_eq!(preamp(&observer), -2.0);
    let end = control.apply_confirm().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Kept, "confirmed"),
        "{end:?}"
    );
    assert_eq!(preamp(&observer), -2.0);
    assert!(hub_active(&control, OTHER).is_null());

    // Timeout on OTHER through the poll.
    observer.set_preamp(-6.0).unwrap();
    control
        .hub_snapshots_recall(OTHER.into(), id.clone(), true, 1500)
        .unwrap();
    assert_eq!(preamp(&observer), -2.0);
    let mut end = control.apply_poll();
    wait_until("the timeout", || {
        end = control.apply_poll();
        end.phase != GuardPhase::Waiting
    });
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "timeout"),
        "{end:?}"
    );
    assert_eq!(preamp(&observer), -6.0);
    assert!(hub_active(&control, OTHER).is_null());

    // The open device changes mid-guard (same session): the guard survives, Keep reaches OTHER.
    control
        .hub_snapshots_recall(OTHER.into(), id.clone(), true, 30_000)
        .unwrap();
    let other_handle = control
        .link_devices()
        .into_iter()
        .find(|d| d.serial == OTHER)
        .unwrap()
        .handle;
    control
        .link_select_device(other_handle, OTHER.into())
        .unwrap();
    let st = control.apply_poll();
    assert_eq!(st.phase, GuardPhase::Waiting, "{st:?}");
    let end = control.apply_confirm().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Kept, "confirmed"),
        "{end:?}"
    );
    assert_eq!(preamp(&observer), -2.0);
    assert!(hub_active(&control, OTHER).is_null());

    // The session drops mid-guard: the hub reverts (session_closed); this core ends Lost.
    observer.set_preamp(-6.0).unwrap();
    control
        .hub_snapshots_recall(OTHER.into(), id.clone(), true, 30_000)
        .unwrap();
    assert_eq!(preamp(&observer), -2.0);
    control.link_close();
    wait_until("the hub's revert", || preamp(&observer) == -6.0);
    let st = control.apply_poll();
    assert_eq!(st.phase, GuardPhase::Lost, "{st:?}");
    assert!(hub_active(&observer, OTHER).is_null());
    observer.link_close();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p2ffix_a_guarded_recall_with_no_bound_device_is_adopted() {
    let dir = scratch("unbound");
    let (hub, _, _) = hub_in(&dir);
    let observer = client(&hub, Some(OPEN));
    let id = seed(&observer, &dir, "obs");
    let unbound = client(&hub, None);
    assert!(unbound.hub_caps().hub_snapshots, "{:?}", unbound.hub_caps());

    // Adopted although no device is bound: the status shows it and Keep reaches the hub.
    let r = unbound
        .hub_snapshots_recall(OPEN.into(), id.clone(), true, 30_000)
        .unwrap();
    assert!(!r.guard.is_empty(), "{r:?}");
    let st = unbound.apply_status();
    assert_eq!(st.phase, GuardPhase::Waiting, "{st:?}");
    assert!(st.hub, "{st:?}");
    assert!(
        !hub_active(&observer, OPEN).is_null(),
        "the hub's guard runs"
    );
    assert_eq!(preamp(&observer), -2.0);
    assert_eq!(unbound.apply_poll().phase, GuardPhase::Waiting);
    let end = unbound.apply_confirm().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Kept, "confirmed"),
        "{end:?}"
    );
    assert!(hub_active(&observer, OPEN).is_null());
    assert_eq!(preamp(&observer), -2.0);

    // And Revert, still unbound: the hub puts the device back.
    observer.set_preamp(-6.0).unwrap();
    unbound
        .hub_snapshots_recall(OPEN.into(), id.clone(), true, 30_000)
        .unwrap();
    assert_eq!(preamp(&observer), -2.0);
    let end = unbound.apply_revert().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "reverted"),
        "{end:?}"
    );
    assert_eq!(preamp(&observer), -6.0);
    assert!(hub_active(&observer, OPEN).is_null());

    // No live session: refused before anything is sent.
    unbound.link_close();
    assert!(
        unbound
            .hub_snapshots_recall(OPEN.into(), id, true, 30_000)
            .is_err()
    );
    assert!(hub_active(&observer, OPEN).is_null(), "nothing began");
    observer.link_close();
    let _ = std::fs::remove_dir_all(&dir);
}
