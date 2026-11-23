//! WP FEATUI (AIVIS's web part): an agent's presence on the real hub, through the core's
//! calls over DSPi Link: a console that runs an agent says so (`hub_presence_set`), another
//! console hears `presence.changed` and reads the list (`hub_presence_list`) with the hub's
//! own label of the session; a Viewer may read and not say; clearing it is heard too.
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

use dspi_core::{DspiCore, DspiError, HubServerEvent, HubServerListener};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role, ServiceConfig};
use dspi_transport::{Capture, FakeDevice, FakeOptions};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "FEATUI-1";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-featui").join(format!(
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
        services: ServiceConfig::default(),
        ..HubConfig::in_process("FEATUI hub")
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
        "wp-featui".into(),
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
fn wp_featui_an_agents_presence_is_said_heard_and_read_on_the_real_hub() {
    let dir = scratch("presence");
    let hub = hub_in(&dir);
    let desk = client(&hub, Role::Control);
    let phone = client(&hub, Role::Viewer);
    assert!(
        phone
            .hub_caps()
            .caps
            .iter()
            .any(|c| c == dspi_core::CAP_PRESENCE),
        "{:?}",
        phone.hub_caps()
    );
    let heard = Arc::new(Heard::default());
    phone.set_hub_server_listener(Some(Arc::clone(&heard) as Arc<dyn HubServerListener>));
    assert!(phone.hub_presence_list().unwrap().is_empty());

    desk.hub_presence_set(
        "Claude".into(),
        SERIAL.into(),
        "Setting a gain on Out 1".into(),
        20,
    )
    .unwrap();
    heard.wait_for("presence.changed", |e| e.kind == "presence.changed");
    let rows = phone.hub_presence_list().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].label, "Claude");
    assert_eq!(rows[0].serial, SERIAL);
    assert_eq!(rows[0].what, "Setting a gain on Out 1");
    assert_eq!(rows[0].kind, "agent");
    assert!(
        !rows[0].by.is_empty() && rows[0].ttl_s > 0 && rows[0].ttl_s <= 20,
        "{rows:?}"
    );
    // JSON for C and wasm: camelCase
    assert_eq!(
        serde_json::to_value(&rows[0]).unwrap()["ttlS"],
        rows[0].ttl_s
    );

    // A Viewer reads, never says
    assert_eq!(
        phone
            .hub_presence_set("Other".into(), String::new(), String::new(), 20)
            .unwrap_err(),
        DspiError::Denied
    );
    // Cleared: heard and gone
    let before = heard.0.lock().unwrap().len();
    desk.hub_presence_set(String::new(), String::new(), String::new(), 0)
        .unwrap();
    wait_until("the second presence.changed", || {
        heard
            .0
            .lock()
            .unwrap()
            .iter()
            .skip(before)
            .any(|e| e.kind == "presence.changed")
    });
    assert!(phone.hub_presence_list().unwrap().is_empty());
}
