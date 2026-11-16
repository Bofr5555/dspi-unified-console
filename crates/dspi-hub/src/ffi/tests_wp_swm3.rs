//! WP SW-M3 tests: the hub's Swift binding over real sockets on loopback, with the
//! core's own Link session as the client, on the demo device. No USB.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

/// A scratch directory: `temp_dir` is under the build cache in the build container.
fn scratch(what: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-swm3-unit").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

fn options(store_dir: &str, access: HubAccess) -> HubStartOptions {
    HubStartOptions {
        name: "SWM3 hub".to_owned(),
        store_dir: store_dir.to_owned(),
        port: 0,
        access,
        usb: false,
        app: false,
        loopback: true,
        web: false,
        mdns_name: None,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Every event, and what a call from inside the listener got.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<HubHostEvent>>,
    /// The host, to try calls from inside the listener.
    host: Mutex<Option<Weak<HubHost>>>,
    /// (stop, begin_pairing refused, status running) from inside, once.
    inside: Mutex<Option<(bool, bool, bool)>>,
}

impl HubHostListener for Recorder {
    fn on_hub_event(&self, event: HubHostEvent) {
        if matches!(event, HubHostEvent::SessionOpened { .. }) {
            let host = lock(&self.host).as_ref().and_then(Weak::upgrade);
            let mut inside = lock(&self.inside);
            if let (Some(host), None) = (host, inside.as_ref()) {
                *inside = Some((
                    host.stop(),
                    matches!(host.begin_pairing(), Err(HubHostError::InCallback)),
                    host.status().running,
                ));
            }
        }
        lock(&self.events).push(event);
    }
}

fn url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}/dspi/v1")
}

#[test]
fn wp_swm3_start_status_demo_and_stop() {
    let dir = scratch("start");
    let host = HubHost::new();
    assert!(!host.status().running);
    assert!(host.devices().is_empty());
    assert_eq!(host.clients(), Err(HubHostError::NotRunning));
    assert_eq!(host.attach_demo(), Err(HubHostError::NotRunning));
    assert!(!host.demo_turn_knob());

    let started = host.start(options(&dir, HubAccess::Pin)).unwrap();
    assert_eq!(started.name, "SWM3 hub");
    assert!(started.port > 0);
    assert!(!started.hub_id.is_empty());
    assert_eq!(started.tokens_revoked, 0);
    assert_eq!(
        host.start(options(&dir, HubAccess::Pin)),
        Err(HubHostError::Running)
    );

    let status = host.status();
    assert!(status.running);
    assert_eq!(status.hub_id, started.hub_id);
    assert_eq!(status.port, started.port);
    assert_eq!(status.access, HubAccess::Pin);
    assert!(!status.pairing_open);

    let handle = host.attach_demo().unwrap();
    let devices = host.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].handle, handle);
    assert!(devices[0].online);
    assert_eq!(devices[0].serial.len(), 16);
    assert!(host.demo_turn_knob());

    let pairing = host.begin_pairing().unwrap();
    assert!(!pairing.pin.is_empty());
    assert!(pairing.expires_in_ms > 0);
    assert!(!format!("{pairing:?}").contains(&pairing.pin));
    let status = host.status();
    assert!(status.pairing_open);
    assert!(status.pairing_expires_in_ms > 0);
    assert!(host.cancel_pairing());
    assert!(!host.status().pairing_open);
    assert_eq!(host.clients(), Ok(Vec::new()));

    assert!(host.stop());
    assert!(!host.status().running);
    assert!(host.stop(), "a second stop is nothing");
    assert!(!host.cancel_pairing());
    assert!(!host.revoke(1));
    assert!(!host.set_role(1, HubClientRole::Admin));
}

#[test]
fn wp_swm3_options_are_checked() {
    let host = HubHost::new();
    let mut bad = options("", HubAccess::Pin);
    assert!(matches!(
        host.start(bad.clone()),
        Err(HubHostError::Invalid { .. })
    ));
    assert!(matches!(
        hub_saved_settings(String::new()),
        Err(HubHostError::Invalid { .. })
    ));
    // A name is cleaned: no control characters, bounded.
    bad.store_dir = scratch("name");
    bad.name = format!("  a\u{7}b{}  ", "x".repeat(100));
    let started = host.start(bad).unwrap();
    assert_eq!(started.name.chars().count(), MAX_HUB_NAME - 2);
    assert!(started.name.starts_with("ab"));
    host.stop();
}

#[test]
fn wp_swm3_open_access_has_nothing_to_pair() {
    let dir = scratch("open");
    let host = HubHost::new();
    host.start(options(&dir, HubAccess::Open)).unwrap();
    assert_eq!(host.status().access, HubAccess::Open);
    assert_eq!(host.begin_pairing(), Err(HubHostError::OpenAccess));
    host.stop();
}

#[test]
fn wp_swm3_a_port_in_use_is_io() {
    let dir = scratch("port");
    let first = HubHost::new();
    let port = first.start(options(&dir, HubAccess::Pin)).unwrap().port;
    let second = HubHost::new();
    let mut o = options(&scratch("port2"), HubAccess::Pin);
    o.port = port;
    assert!(matches!(second.start(o), Err(HubHostError::Io { .. })));
    assert!(!second.status().running);
    first.stop();
}

/// A client pairs with the window's PIN, and the hub's side sees it, changes its
/// role and revokes it; the store keeps it across a stop; a switch of access mode
/// says beforehand what it costs and then costs it.
#[test]
fn wp_swm3_pairing_roles_revoke_and_saved_settings() {
    let dir = scratch("pair");
    let host = HubHost::new();
    let recorder = Arc::new(Recorder::default());
    *lock(&recorder.host) = Some(Arc::downgrade(&host));
    assert!(host.set_listener(Some(Arc::clone(&recorder) as Arc<dyn HubHostListener>)));
    let started = host.start(options(&dir, HubAccess::Pin)).unwrap();
    host.attach_demo().unwrap();
    let pin = host.begin_pairing().unwrap().pin;

    let client = DspiCore::new();
    let hello = client
        .link_connect(
            url(started.port),
            "swm3 client".into(),
            "0".into(),
            String::new(),
            started.hub_id.clone(),
        )
        .unwrap();
    assert!(hello.needs_pairing);
    assert_eq!(hello.hub_id, started.hub_id);
    let token = client.link_pair(pin).unwrap();
    assert!(!token.is_empty());
    wait_until("the client's session", || {
        lock(&recorder.events)
            .iter()
            .any(|e| matches!(e, HubHostEvent::SessionOpened { .. }))
    });
    // Inside the listener: a stop and a pairing are refused, a read is not.
    assert_eq!(*lock(&recorder.inside), Some((false, true, true)));
    assert!(host.status().running);
    assert!(!host.status().pairing_open, "the right PIN ends the window");

    wait_until("the client online", || {
        host.clients()
            .is_ok_and(|c| c.len() == 1 && c[0].online && c[0].name == "swm3 client")
    });
    let paired = host.clients().unwrap().remove(0);
    assert_eq!(paired.role, HubClientRole::Control, "a PIN gives control");
    assert!(paired.created > 0);
    assert!(host.set_role(paired.cid, HubClientRole::Admin));
    assert_eq!(host.clients().unwrap()[0].role, HubClientRole::Admin);
    assert!(!host.set_role(paired.cid + 1000, HubClientRole::Viewer));

    client.link_close();
    host.stop();
    assert!(lock(&recorder.events).contains(&HubHostEvent::Stopped));

    // The store, with no hub on it.
    let saved = hub_saved_settings(dir.clone()).unwrap();
    assert!(saved.exists);
    assert_eq!(saved.hub_id.as_deref(), Some(started.hub_id.as_str()));
    assert_eq!(saved.name.as_deref(), Some("SWM3 hub"));
    assert_eq!(saved.access, Some(HubAccess::Pin));
    assert_eq!(saved.default_port, crate::DEFAULT_PORT);
    assert_eq!(saved.clients.len(), 1);
    assert_eq!(saved.clients[0].role, HubClientRole::Admin);
    assert!(!saved.clients[0].online);
    assert_eq!(saved.tokens_revoked_if_pin, 0);
    assert_eq!(saved.tokens_revoked_if_open, 1);

    // A start with the stored name (""), then a revoke.
    let mut o = options(&dir, HubAccess::Pin);
    o.name = String::new();
    let again = host.start(o).unwrap();
    assert_eq!(again.name, "SWM3 hub");
    assert_eq!(again.hub_id, started.hub_id);
    assert!(host.revoke(paired.cid));
    assert_eq!(host.clients(), Ok(Vec::new()));
    assert!(!host.revoke(paired.cid));
    host.stop();

    // Open access costs every token; none is left here, so pair one more first.
    let fresh = scratch("switch");
    let started = host.start(options(&fresh, HubAccess::Pin)).unwrap();
    let pin = host.begin_pairing().unwrap().pin;
    let client = DspiCore::new();
    client
        .link_connect(
            url(started.port),
            "two".into(),
            "0".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
    client.link_pair(pin).unwrap();
    client.link_close();
    host.stop();
    assert_eq!(
        hub_saved_settings(fresh.clone())
            .unwrap()
            .tokens_revoked_if_open,
        1
    );
    let open = host.start(options(&fresh, HubAccess::Open)).unwrap();
    assert_eq!(open.tokens_revoked, 1);
    assert_eq!(host.status().tokens_revoked, 1);
    host.stop();
    assert!(host.set_listener(None));
}

/// The app's core on the hub: a second app is refused, and the hub's stop gives the
/// core its own path back.
#[test]
fn wp_swm3_attach_app() {
    let dir = scratch("app");
    let host = HubHost::new();
    let core = DspiCore::new();
    assert_eq!(
        host.attach_app(Arc::clone(&core)),
        Err(HubHostError::NotRunning)
    );
    let mut o = options(&dir, HubAccess::Pin);
    o.app = true;
    host.start(o).unwrap();
    host.attach_app(Arc::clone(&core)).unwrap();
    // The core's devices are the hub's now: the demo device the hub serves.
    host.attach_demo().unwrap();
    wait_until("the hub's device in the core's list", || {
        core.scan_devices().len() == 1
    });
    let other = DspiCore::new();
    assert!(matches!(
        host.attach_app(other),
        Err(HubHostError::AppAttached { .. })
    ));
    let serial = core.scan_devices()[0].serial.clone();
    core.select_device(serial).unwrap();
    assert!(core.is_connected());
    host.stop();
    // Back on its own path: the hub's device has gone with the hub.
    assert!(!core.is_connected());
    assert!(
        core.scan_devices()
            .iter()
            .all(|d| d.serial != "DEMO0000DSPI0001")
    );
}

/// Dropping the object stops its hub.
#[test]
fn wp_swm3_drop_stops_the_hub() {
    let dir = scratch("drop");
    let host = HubHost::new();
    let port = host.start(options(&dir, HubAccess::Pin)).unwrap().port;
    drop(host);
    let again = HubHost::new();
    let mut o = options(&scratch("drop2"), HubAccess::Pin);
    o.port = port;
    again.start(o).expect("the port is free again");
    again.stop();
}
