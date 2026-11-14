//! WP P1QT1 (HWLK1F's note): a client's Save of the System window gets the
//! hub's `system.set` warnings through the core, not only the revision: a
//! unit whose firmware has no audio link is kept in the model and named.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_core::link::AlinkSystemEdits;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AlinkSetup;
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p1qt1").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// A hub with a sender, one receiver and the plain demo device (no audio
/// link) as unit K, all three in the model.
fn hub() -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("P1QT1 hub")
    })
    .unwrap();
    for (serial, role) in [("Q-TX", AlinkRole::Sender), ("Q-A", AlinkRole::Receiver)] {
        let setup = AlinkSetup {
            role,
            ..AlinkSetup::default()
        };
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    hub.attach(Box::new(FakeDevice::demo().with_serial("Q-K")), 0)
        .unwrap();
    let model = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "Q-TX"}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "Q-A", "room": "main", "upstream": "tx"},
            {"id": "K", "name": "K340", "serial": "Q-K", "room": "main", "upstream": "tx"}
        ],
        "scenes": []
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    hub
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-p1qt1".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core
}

fn revision(core: &DspiCore) -> u64 {
    let start = Instant::now();
    loop {
        let v = core.alink_system_view().unwrap();
        if !v.stale {
            return v.revision;
        }
        assert!(start.elapsed() < WAIT, "always stale");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn wp_p1qt1_a_save_reports_the_hubs_warnings() {
    let hub = hub();
    let core = connect(&hub);
    let rev = revision(&core);
    let saved = core
        .alink_system_save_report(rev, AlinkSystemEdits::default())
        .unwrap();
    assert_eq!(saved.revision, rev + 1);
    assert_eq!(saved.warnings.len(), 1, "{:?}", saved.warnings);
    assert!(
        saved.warnings[0].starts_with("K340 (Q-K): its firmware has no audio link"),
        "{:?}",
        saved.warnings
    );
    // The revision-only call is the same save.
    let rev = revision(&core);
    assert_eq!(
        core.alink_system_save(rev, AlinkSystemEdits::default())
            .unwrap(),
        rev + 1
    );
    hub.stop();
}

#[test]
fn wp_p1qt1_a_clean_model_saves_without_warnings() {
    let hub = hub();
    let core = connect(&hub);
    // K leaves the model: nothing left to warn about.
    let model = json!({
        "senders": [{"id": "tx", "name": "Sender", "serial": "Q-TX"}],
        "units": [{"id": "A", "name": "Main left", "serial": "Q-A", "upstream": "tx"}],
        "scenes": []
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let rev = revision(&core);
    let saved = core
        .alink_system_save_report(rev, AlinkSystemEdits::default())
        .unwrap();
    assert!(saved.warnings.is_empty(), "{:?}", saved.warnings);
    hub.stop();
}
