//! WP P3HB: `DspiCore::hub_health` carries the board's three facts from the real hub:
//! the stream counters, the clip events of the last minute on a device that sends
//! them, and the last change with the name of the client that made it (a core paired
//! with the hub's PIN, writing the master volume).
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

use dspi_core::{DspiCore, HubHealth};
use dspi_hub::{
    AccessMode, HealthConfig, HistoryConfig, Hub, HubConfig, NetworkConfig, ServiceConfig,
};
use dspi_proto::ext::NOTIFY_EVT_EXT_CLIP;
use dspi_transport::{Capture, FakeDevice, FakeOptions, FakeRemote};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "P3HB-CORE";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3hb-core").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn hub_in(dir: &std::path::Path) -> (Hub, FakeRemote) {
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
            health: Some(HealthConfig {
                status_every: Duration::from_millis(200),
                counters_every: Duration::from_millis(300),
                ..HealthConfig::default()
            }),
            history: Some(HistoryConfig::default()),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P3HB hub")
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
    let remote = device.remote();
    hub.attach(Box::new(device), 0).unwrap();
    (hub, remote)
}

/// A core paired on `hub` with the hub's PIN (Control), bound to the device.
fn client(hub: &Hub) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-p3hb".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

fn record(core: &DspiCore) -> Option<HubHealth> {
    core.hub_health()
        .unwrap()
        .into_iter()
        .find(|h| h.serial == SERIAL)
}

#[test]
fn wp_p3hb_hub_health_carries_counters_clips_and_the_last_change() {
    let dir = scratch("core");
    let (hub, remote) = hub_in(&dir);
    let core = client(&hub);
    // The fake's counters read 0: present, and zero.
    wait_until("the counters", || {
        record(&core)
            .and_then(|h| h.counters)
            .is_some_and(|c| c.underruns.is_some() && c.starvation.is_some())
    });
    let c = record(&core).unwrap().counters.unwrap();
    assert_eq!((c.underruns, c.starvation), (Some(0), Some(0)));
    // Clip events from the device.
    wait_until("clip events announced", || {
        record(&core).is_some_and(|h| h.clip_per_min == Some(0))
    });
    for ch in 0..3u8 {
        let mut p = vec![2, NOTIFY_EVT_EXT_CLIP, 0, 0, ch, 1, 0, 0];
        p.extend((1u32 << ch).to_le_bytes());
        remote.push(p);
    }
    wait_until("three clip events", || {
        record(&core).is_some_and(|h| h.clip_per_min == Some(3))
    });
    // A write by this client: the last change names it.
    core.set_master_volume(-21.0).unwrap();
    let name = hub
        .network()
        .unwrap()
        .auth()
        .clients()
        .into_iter()
        .max_by_key(|c| c.cid)
        .unwrap()
        .name;
    assert!(!name.is_empty());
    wait_until("the last change", || {
        record(&core)
            .and_then(|h| h.last_change)
            .is_some_and(|lc| lc.by == name)
    });
    let lc = record(&core).unwrap().last_change.unwrap();
    assert!(lc.at.ends_with('Z') && !lc.text.is_empty(), "{lc:?}");
    core.link_close();
    hub.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
