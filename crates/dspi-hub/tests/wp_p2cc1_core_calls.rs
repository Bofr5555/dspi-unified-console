//! WP P2CC1: the core's typed calls for the hub's own families
//! (`dspi_core::hub_server`) against the real hub: the inventory, Identify,
//! health, statistics, the history and its `event.added`, the snapshot store
//! with `snapshot.changed`, recall on the hub, and the merged list. The client
//! is the real one (`DspiCore` over DSPi Link), paired with the hub's PIN
//! (Control), and given Viewer or Admin on the hub's machine where a test
//! needs it. The calls without a hub, and the wire shapes against the fake
//! hub, are in `crates/dspi-core/src/hub_server/tests_wp_p2cc1.rs`.
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

use dspi_core::hub_server::{SNAPSHOT_WIRE_MAX_BYTES, WHERE_BOTH, WHERE_HUB, WHERE_LOCAL};
use dspi_core::{DspiCore, DspiError, HubServerEvent, HubServerListener};
use dspi_hub::{
    AccessMode, HealthConfig, HistoryConfig, Hub, HubConfig, NetworkConfig, Role, ServiceConfig,
};
use dspi_transport::{Capture, FakeDevice, FakeOptions};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "P2CC1-1";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2cc1").join(format!(
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

/// A hub with every family of these calls on, and one wp/FWX device (it has
/// `EXT_IDENTIFY`).
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
            inventory: true,
            identify: true,
            health: Some(HealthConfig::default()),
            history: Some(HistoryConfig::default()),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2CC1 hub")
    })
    .unwrap();
    let device = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
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
        "wp-p2cc1".into(),
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
    core.fetch_all().unwrap();
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
fn wp_p2cc1_the_reads_identify_and_the_history_on_the_real_hub() {
    let dir = scratch("reads");
    let hub = hub_in(&dir);
    let control = client(&hub, Role::Control);
    let caps = control.hub_caps();
    assert!(caps.on_hub, "{caps:?}");
    assert!(
        caps.inventory
            && caps.identify
            && caps.health
            && caps.events
            && caps.hub_snapshots
            && caps.hub_stats
            && caps.param_api,
        "{caps:?}"
    );

    let devices = control.hub_inventory().unwrap();
    let d = devices.iter().find(|d| d.serial == SERIAL).unwrap();
    assert!(d.online && d.handle.is_some(), "{d:?}");
    assert!(!d.first_seen.is_empty() && !d.platform.is_empty(), "{d:?}");

    wait_until("a health record", || {
        control
            .hub_health()
            .unwrap()
            .iter()
            .any(|h| h.serial == SERIAL && h.cpu_pct.is_some())
    });

    // Statistics are an Admin's.
    assert_eq!(control.hub_stats().unwrap_err(), DspiError::Denied);
    let admin = client(&hub, Role::Admin);
    let stats = admin.hub_stats().unwrap();
    assert!(stats.sessions >= 2, "{stats:?}");
    assert!(
        stats.devices.iter().any(|d| d.serial == SERIAL),
        "{stats:?}"
    );

    // The watch first, so that Identify's record comes back as event.added.
    let heard = Arc::new(Heard::default());
    control.set_hub_server_listener(Some(heard.clone()));
    control.hub_events_watch(true, SERIAL.into()).unwrap();

    // Identify on the hub: the hub's message (Control may), its answer the
    // device's state; again within 2 s is the hub's rate limit, typed.
    let st = control.hub_identify(5, None).unwrap().expect("the status");
    assert!((4..=5).contains(&st.remaining_s), "{st:?}");
    assert_eq!(st.chip, "RP2350");
    let e = control.hub_identify(5, None).unwrap_err();
    assert!(
        matches!(&e, DspiError::Link { code, .. } if code == "rate_limited"),
        "{e:?}"
    );
    assert_eq!(e.notice_key(), "dlink.rate_limited");

    let added = heard.wait_for("identify's record", |e| {
        e.kind == "event.added" && e.record.as_ref().is_some_and(|r| r.source == "identify")
    });
    let rec = added.record.unwrap();
    assert_eq!(rec.by, "wp-p2cc1", "{rec:?}");
    assert_eq!(added.serial, SERIAL);

    let page = control.hub_events_read(String::new(), 0, 100).unwrap();
    assert_eq!(page.serial, SERIAL);
    assert!(page.newest_id >= rec.id, "{page:?}");
    assert!(
        page.records
            .iter()
            .any(|r| r.source == "identify" && r.by == "wp-p2cc1"),
        "{page:?}"
    );
    let e = control
        .hub_events_read("NO-SUCH-DEVICE".into(), 0, 10)
        .unwrap_err();
    assert!(
        matches!(&e, DspiError::Link { code, .. } if code == "not_found"),
        "{e:?}"
    );

    // A Viewer reads, and Identify is refused it.
    let viewer = client(&hub, Role::Viewer);
    assert!(!viewer.hub_inventory().unwrap().is_empty());
    assert_eq!(viewer.hub_identify(1, None).unwrap_err(), DspiError::Denied);
    for c in [&control, &admin, &viewer] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wp_p2cc1_the_hubs_snapshots_and_the_merged_list_on_the_real_hub() {
    let dir = scratch("snapshots");
    let hub = hub_in(&dir);
    let control = client(&hub, Role::Control);
    let heard = Arc::new(Heard::default());
    control.set_hub_server_listener(Some(heard.clone()));
    let local = dir.join("console-snapshots");
    control
        .snapshot_set_dir(local.to_string_lossy().into_owned())
        .unwrap();

    // This console's own snapshots of the device.
    let a = control
        .snapshot_capture("Both sides".into(), String::new())
        .unwrap();
    let b = control
        .snapshot_capture("Mine".into(), String::new())
        .unwrap();
    assert_eq!(a.serial, SERIAL);

    // "Save to the hub", twice: the second is the same snapshot.
    let put = control.hub_snapshots_save(a.id.clone()).unwrap();
    assert!(put.created);
    assert_eq!(put.snapshot.id, a.id);
    assert!(!control.hub_snapshots_save(a.id.clone()).unwrap().created);
    let changed = heard.wait_for("snapshot.changed", |e| {
        e.kind == "snapshot.changed" && e.snapshot == a.id
    });
    assert_eq!(
        (changed.change.as_str(), changed.serial.as_str()),
        ("put", SERIAL)
    );
    // `b` on the hub too, then renamed there: the same id, differing content.
    control.hub_snapshots_save(b.id.clone()).unwrap();
    let renamed = control
        .hub_snapshots_rename(b.id.clone(), "Renamed on the hub".into())
        .unwrap();
    assert_eq!(renamed.name, "Renamed on the hub");
    let noted = control
        .hub_snapshots_note(b.id.clone(), "a note".into())
        .unwrap();
    assert_eq!(noted.note, "a note");

    let list = control.hub_snapshots_list(String::new()).unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    let got = control.hub_snapshots_get(a.id.clone()).unwrap();
    assert_eq!(got.snapshot.id, a.id);
    assert!(serde_json::from_str::<Value>(&got.document).is_ok());

    // Both sides in one list, the hub's winning on `b`.
    let merged = control.snapshot_list_merged(String::new());
    assert!(merged.hub_listed, "{merged:?}");
    let rows: Vec<(String, &str, bool)> = merged
        .entries
        .iter()
        .map(|r| (r.snapshot.id.clone(), r.location.as_str(), r.local_copy))
        .collect();
    assert_eq!(
        rows,
        [
            (a.id.clone(), WHERE_BOTH, false),
            (b.id.clone(), WHERE_HUB, false),
            (b.id.clone(), WHERE_LOCAL, true),
        ]
    );
    assert_eq!(merged.entries[1].snapshot.name, "Renamed on the hub");
    assert_eq!(merged.entries[2].snapshot.name, "Mine");

    // "Copy here" keeps this console's own `b`: nothing overwritten.
    let copy = control.hub_snapshots_copy_here(b.id.clone()).unwrap();
    assert_eq!(copy.name, "Renamed on the hub");
    assert_ne!(copy.id, b.id);
    let mine = control.snapshot_list(SERIAL.into());
    assert!(
        mine.iter().any(|m| m.id == b.id && m.name == "Mine"),
        "{mine:?}"
    );
    // The same snapshot copied again is answered as it is.
    assert_eq!(control.hub_snapshots_copy_here(a.id.clone()).unwrap(), a);

    // Recall on the hub, written as this session.
    let r = control
        .hub_snapshots_recall(String::new(), a.id.clone(), false, 0)
        .unwrap();
    assert_eq!(
        (r.serial.as_str(), r.snapshot.as_str()),
        (SERIAL, a.id.as_str())
    );
    assert!(r.image, "{r:?}");

    // The wire cap: the local refusal is the hub's own answer, word for word.
    let mut doc: Value = serde_json::from_str(&got.document).unwrap();
    doc["snapshot"]["id"] = json!("0000000000042");
    // Just over the cap, compact, so that the hub's own answer fits one frame.
    let base = doc.to_string().len() + r#","pad":"""#.len();
    assert!(base < SNAPSHOT_WIRE_MAX_BYTES, "{base}");
    doc["pad"] = json!("x".repeat(SNAPSHOT_WIRE_MAX_BYTES + 100 - base));
    let len = doc.to_string().len();
    assert_eq!(len, SNAPSHOT_WIRE_MAX_BYTES + 100);
    assert!(
        len < 65_536 - 512,
        "fits one frame for the hub's own answer: {len}"
    );
    let local_refusal = control
        .hub_snapshots_put(String::new(), doc.to_string())
        .unwrap_err();
    let mut f = serde_json::Map::new();
    f.insert("serial".into(), SERIAL.into());
    f.insert("document".into(), doc);
    let DspiError::Link { code, msg } = control.link_hub_request("snapshot.put", f).unwrap_err()
    else {
        panic!("not the hub's refusal");
    };
    assert_eq!(code, "conflict");
    assert_eq!(
        local_refusal,
        DspiError::Link {
            code,
            msg: msg.trim_start_matches("hub refused: conflict ").to_owned()
        }
    );

    // Delete is an Admin's; a Viewer changes nothing.
    assert_eq!(
        control.hub_snapshots_delete(a.id.clone()).unwrap_err(),
        DspiError::Denied
    );
    let viewer = client(&hub, Role::Viewer);
    assert_eq!(
        viewer
            .hub_snapshots_rename(a.id.clone(), "no".into())
            .unwrap_err(),
        DspiError::Denied
    );
    let admin = client(&hub, Role::Admin);
    admin.hub_snapshots_delete(a.id.clone()).unwrap();
    let e = control.hub_snapshots_get(a.id.clone()).unwrap_err();
    assert!(
        matches!(&e, DspiError::Link { code, .. } if code == "not_found"),
        "{e:?}"
    );
    // This console's own copy stays.
    assert!(
        control
            .snapshot_list(SERIAL.into())
            .iter()
            .any(|m| m.id == a.id)
    );
    for c in [&control, &admin, &viewer] {
        c.link_close();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
