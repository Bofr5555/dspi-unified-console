//! WP P2CC3: the core's typed calls for the hub's device and name writes
//! (`dspi_core::hub_server::manage`) against the real hub: `inventory.set`,
//! `inventory.forget` (refused while online), `hub.rename`, and
//! `device.identify` for a device other than the open one. The client is the
//! real one (`DspiCore` over DSPi Link), paired with the hub's PIN (Control)
//! and given Viewer or Admin on the hub's machine where a test needs it.
//! The calls without a hub are in
//! `crates/dspi-core/src/hub_server/tests_wp_p2cc3.rs`.
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

use dspi_core::{DspiCore, DspiError};
use dspi_hub::{AccessMode, HistoryConfig, Hub, HubConfig, NetworkConfig, Role, ServiceConfig};
use dspi_transport::{Capture, FakeDevice, FakeOptions};

const WAIT: Duration = Duration::from_secs(20);
/// The device every client opens.
const OPEN: &str = "P2CC3-1";
/// Another device on the hub, never opened here.
const OTHER: &str = "P2CC3-2";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2cc3").join(format!(
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

fn link_code(e: &DspiError) -> &str {
    match e {
        DspiError::Link { code, .. } => code,
        _ => "",
    }
}

/// A wp/FWX device (it has `EXT_IDENTIFY`).
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

/// A hub with `services`, and two devices: their handles.
fn hub_in(dir: &std::path::Path, services: ServiceConfig) -> (Hub, u8, u8) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        services,
        ..HubConfig::in_process("P2CC3 hub")
    })
    .unwrap();
    let open = hub.attach(Box::new(device(OPEN)), 0).unwrap();
    let other = hub.attach(Box::new(device(OTHER)), 0).unwrap();
    (hub, open, other)
}

fn every_service() -> ServiceConfig {
    ServiceConfig {
        inventory: true,
        identify: true,
        history: Some(HistoryConfig::default()),
        ..ServiceConfig::default()
    }
}

/// A core paired on `hub` with the hub's PIN (Control), given `role` on the
/// hub's machine, bound to [`OPEN`].
fn client(hub: &Hub, role: Role) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-p2cc3".into(),
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
    wait_until("the device list", || core.link_devices().len() == 2);
    let d = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == OPEN)
        .unwrap();
    core.link_select_device(d.handle, d.serial).unwrap();
    core
}

#[test]
fn wp_p2cc3_inventory_set_and_forget_on_the_real_hub() {
    let dir = scratch("inventory");
    let (hub, _, other) = hub_in(&dir, every_service());
    let admin = client(&hub, Role::Admin);
    let control = client(&hub, Role::Control);

    // Each field replaced; the record comes back as the hub now has it.
    let d = admin
        .hub_inventory_set(
            OTHER.into(),
            Some("Hall B — rack 2".into()),
            Some("Left array\nchecked".into()),
            Some(vec!["main".into(), " left ".into(), "main".into()]),
        )
        .unwrap();
    assert_eq!(d.serial, OTHER);
    assert_eq!(d.location, "Hall B — rack 2");
    assert_eq!(d.note, "Left array\nchecked");
    assert_eq!(d.tags, ["main", "left"], "trimmed, duplicates dropped");
    assert!(d.online, "{d:?}");
    let listed = admin.hub_inventory().unwrap();
    assert_eq!(
        listed.iter().find(|r| r.serial == OTHER).unwrap().location,
        "Hall B — rack 2"
    );
    // None keeps a field; "" and [] clear it. "" is the open device.
    let d = admin
        .hub_inventory_set(OTHER.into(), Some(String::new()), None, Some(Vec::new()))
        .unwrap();
    assert_eq!(
        (d.location.as_str(), d.note.as_str()),
        ("", "Left array\nchecked")
    );
    assert!(d.tags.is_empty(), "{d:?}");
    let d = admin
        .hub_inventory_set(String::new(), None, Some("the open one".into()), None)
        .unwrap();
    assert_eq!((d.serial.as_str(), d.note.as_str()), (OPEN, "the open one"));

    // Past a bound: the hub's bad_request, typed, and nothing changed.
    let e = admin
        .hub_inventory_set(OTHER.into(), Some("x".repeat(257)), None, None)
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let tags: Vec<String> = (0..17).map(|i| format!("t{i}")).collect();
    let e = admin
        .hub_inventory_set(OTHER.into(), None, None, Some(tags))
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let e = admin
        .hub_inventory_set("NO-SUCH-DEVICE".into(), Some("x".into()), None, None)
        .unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    assert_eq!(e.notice_key(), "dlink.not_found");

    // Admin only.
    assert_eq!(
        control
            .hub_inventory_set(OTHER.into(), Some("x".into()), None, None)
            .unwrap_err(),
        DspiError::Denied
    );
    assert_eq!(
        control
            .hub_inventory_forget(OTHER.into(), false)
            .unwrap_err(),
        DspiError::Denied
    );

    // Identify writes a history record of the other device, so a forget has
    // its events to remove too.
    control.hub_identify_serial(OTHER.into(), 1, None).unwrap();

    // Refused while the device is online, by the hub.
    let e = admin.hub_inventory_forget(OTHER.into(), false).unwrap_err();
    assert_eq!(link_code(&e), "conflict", "{e:?}");
    assert_eq!(e.notice_key(), "dlink.conflict");

    // Offline: forgotten, record and history; then not known at all.
    hub.detach(other).unwrap();
    wait_until("the inventory's offline record", || {
        admin
            .hub_inventory()
            .unwrap()
            .iter()
            .any(|r| r.serial == OTHER && !r.online)
    });
    let forgot = admin.hub_inventory_forget(OTHER.into(), true).unwrap();
    assert_eq!(forgot.serial, OTHER);
    assert_eq!(forgot.forgot.first().map(String::as_str), Some("record"));
    assert!(forgot.forgot.iter().any(|f| f == "events"), "{forgot:?}");
    assert!(
        !admin
            .hub_inventory()
            .unwrap()
            .iter()
            .any(|r| r.serial == OTHER)
    );
    let e = admin.hub_inventory_forget(OTHER.into(), false).unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    for c in [&admin, &control] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p2cc3_rename_and_identify_by_serial_on_the_real_hub() {
    let dir = scratch("rename");
    let (hub, _, other) = hub_in(&dir, every_service());
    let admin = client(&hub, Role::Admin);
    let control = client(&hub, Role::Control);
    let viewer = client(&hub, Role::Viewer);

    // The hub's name: cleaned by the hub, in use at once.
    assert_eq!(
        admin.hub_rename("  Studio\u{7}Mac  ".into()).unwrap(),
        "StudioMac"
    );
    assert_eq!(hub.name(), "StudioMac");
    assert_eq!(admin.hub_rename("Studio Mac".into()).unwrap(), "Studio Mac");
    for bad in [String::new(), " \t ".into(), "x".repeat(65)] {
        let e = admin.hub_rename(bad).unwrap_err();
        assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    }
    assert_eq!(hub.name(), "Studio Mac", "nothing changed");
    assert_eq!(
        control.hub_rename("x".into()).unwrap_err(),
        DspiError::Denied
    );

    // Identify a device that is not the open one: the hub's answer is that
    // device's state; again within 2 s is the hub's rate limit, typed.
    let st = control
        .hub_identify_serial(OTHER.into(), 5, None)
        .unwrap()
        .expect("the status");
    assert!((4..=5).contains(&st.remaining_s), "{st:?}");
    assert_eq!(st.chip, "RP2350");
    let e = control
        .hub_identify_serial(OTHER.into(), 5, None)
        .unwrap_err();
    assert_eq!(link_code(&e), "rate_limited", "{e:?}");
    assert_eq!(e.notice_key(), "dlink.rate_limited");
    // The open device is another device: not limited by that.
    let st = control
        .hub_identify_serial(String::new(), 0, None)
        .unwrap()
        .expect("the status");
    assert_eq!(st.remaining_s, 0, "0 stops it");
    // A tone output the device does not have.
    std::thread::sleep(Duration::from_millis(2100));
    let e = control
        .hub_identify_serial(OTHER.into(), 1, Some(99))
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let e = control
        .hub_identify_serial("NO-SUCH-DEVICE".into(), 1, None)
        .unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    assert_eq!(
        viewer
            .hub_identify_serial(OTHER.into(), 1, None)
            .unwrap_err(),
        DspiError::Denied
    );

    // Read-only mode: all four refused, nothing sent (a later Identify is not
    // rate limited, the name and the record are as they were).
    std::thread::sleep(Duration::from_millis(2100));
    admin.set_client_read_only(true).unwrap();
    let r = DspiError::ReadOnly;
    assert_eq!(
        admin
            .hub_inventory_set(OTHER.into(), Some("ro".into()), None, None)
            .unwrap_err(),
        r
    );
    assert_eq!(
        admin.hub_inventory_forget(OTHER.into(), false).unwrap_err(),
        r
    );
    assert_eq!(admin.hub_rename("Read only".into()).unwrap_err(), r);
    assert_eq!(
        admin
            .hub_identify_serial(OTHER.into(), 5, None)
            .unwrap_err(),
        r
    );
    // Reads still go out.
    let rec = admin.hub_inventory().unwrap();
    assert_eq!(rec.iter().find(|d| d.serial == OTHER).unwrap().location, "");
    admin.set_client_read_only(false).unwrap();
    assert_eq!(hub.name(), "Studio Mac");
    admin
        .hub_identify_serial(OTHER.into(), 1, None)
        .expect("not rate limited: the read-only one was never sent");

    // Known but not attached.
    hub.detach(other).unwrap();
    wait_until("the device to go", || {
        control.link_devices().iter().all(|d| d.serial != OTHER)
            || hub
                .devices()
                .iter()
                .any(|d| d.identity.serial == OTHER && d.state != dspi_hub::DeviceState::Online)
    });
    let e = control
        .hub_identify_serial(OTHER.into(), 1, None)
        .unwrap_err();
    assert_eq!(link_code(&e), "offline", "{e:?}");
    for c in [&admin, &control, &viewer] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p2cc3_without_the_caps_every_call_is_unsupported() {
    let dir = scratch("nocaps");
    let (hub, _, _) = hub_in(
        &dir,
        ServiceConfig {
            stats: false,
            ..ServiceConfig::default()
        },
    );
    let admin = client(&hub, Role::Admin);
    let caps = admin.hub_caps();
    assert!(caps.on_hub, "{caps:?}");
    assert!(
        !caps.inventory && !caps.identify && !caps.hub_stats,
        "{caps:?}"
    );
    let u = DspiError::Unsupported;
    assert_eq!(
        admin
            .hub_inventory_set(OTHER.into(), Some("x".into()), None, None)
            .unwrap_err(),
        u
    );
    assert_eq!(
        admin.hub_inventory_forget(OTHER.into(), false).unwrap_err(),
        u
    );
    assert_eq!(admin.hub_rename("x".into()).unwrap_err(), u);
    assert_eq!(
        admin
            .hub_identify_serial(OTHER.into(), 1, None)
            .unwrap_err(),
        u
    );
    assert_eq!(hub.name(), "P2CC3 hub");
    admin.link_close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The open device's preamp as the device has it now.
#[cfg(feature = "api")]
fn preamp(core: &DspiCore) -> f32 {
    core.fetch_all().unwrap();
    core.get_state().preamp_db
}

/// Hub-side recall is the parameter API's (feature `api`).
#[cfg(feature = "api")]
#[test]
fn wp_p2cc3_a_guarded_recall_on_the_hub_is_the_cores_guarded_apply() {
    use dspi_core::GuardPhase;
    let dir = scratch("guard");
    let (hub, _, _) = hub_in(&dir, every_service());
    let control = client(&hub, Role::Control);
    control
        .snapshot_set_dir(dir.join("console").to_string_lossy().into_owned())
        .unwrap();
    control.fetch_all().unwrap();
    control.set_preamp(-2.0).unwrap();
    let snap = control
        .snapshot_capture("At -2".into(), String::new())
        .unwrap();
    control.hub_snapshots_save(snap.id.clone()).unwrap();
    control.set_preamp(-6.0).unwrap();
    assert_eq!(preamp(&control), -6.0);

    // Revert: the hub puts back what it captured, as this session.
    let r = control
        .hub_snapshots_recall(String::new(), snap.id.clone(), true, 30_000)
        .unwrap();
    assert!(!r.guard.is_empty(), "{r:?}");
    let st = control.apply_status();
    assert_eq!(st.phase, GuardPhase::Waiting, "{st:?}");
    assert!(st.hub, "{st:?}");
    assert_eq!(st.label, "At -2");
    assert_eq!(st.timeout_ms, 30_000);
    assert!(st.remaining_ms > 20_000, "{st:?}");
    assert_eq!(preamp(&control), -2.0);
    // One guard at a time: another guarded recall, or a begin here, is refused
    // with nothing sent.
    let e = control
        .hub_snapshots_recall(String::new(), snap.id.clone(), true, 0)
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    assert!(matches!(
        control.apply_guard_begin("x".into(), 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(control.apply_poll().phase, GuardPhase::Waiting);
    let end = control.apply_revert().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "reverted"),
        "{end:?}"
    );
    assert_eq!(preamp(&control), -6.0);
    let hub_says = control.link_hub_request("apply.status", {
        let mut f = serde_json::Map::new();
        let open = control
            .link_devices()
            .into_iter()
            .find(|d| d.serial == OPEN);
        f.insert("handle".into(), open.unwrap().handle.into());
        f
    });
    assert!(
        hub_says.as_ref().is_ok_and(|v| v["active"].is_null()),
        "{hub_says:?}"
    );

    // Keep: the recalled settings stay.
    control
        .hub_snapshots_recall(String::new(), snap.id.clone(), true, 0)
        .unwrap();
    assert_eq!(control.apply_status().timeout_ms, 30_000, "the default");
    let end = control.apply_confirm().unwrap();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Kept, "confirmed"),
        "{end:?}"
    );
    assert_eq!(control.apply_status().phase, GuardPhase::Kept);
    assert_eq!(preamp(&control), -2.0);
    assert!(control.apply_confirm().is_err(), "nothing waits any more");

    // The timeout: the poll ends it, and the settings from before are back.
    control.set_preamp(-6.0).unwrap();
    control
        .hub_snapshots_recall(String::new(), snap.id.clone(), true, 1500)
        .unwrap();
    assert_eq!(preamp(&control), -2.0);
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
    assert_eq!(preamp(&control), -6.0);

    // A recall without the guard is no guarded apply.
    control
        .hub_snapshots_recall(String::new(), snap.id.clone(), false, 0)
        .unwrap();
    assert_eq!(control.apply_status().id, end.id);
    assert_ne!(control.apply_status().phase, GuardPhase::Waiting);
    control.link_close();
    let _ = std::fs::remove_dir_all(&dir);
}
