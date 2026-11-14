//! WP P1SA through a running hub: the core's guarded apply over DSPi Link, with
//! P1HA's guard on the hub. The client is the real one (`DspiCore` over Link),
//! the server the real one.
//!
//! - Keep keeps: the hub's guard ends `confirmed`.
//! - A client that polls reverts by itself at its deadline, then ends the hub's
//!   guard (`cancelled`, `reverted`).
//! - A silent client is reverted by the hub at the hub's deadline; the client's
//!   next poll says so and writes nothing.
//! - A cut connection is reverted by the hub; the client says so after the
//!   reconnect, and nothing is replayed (no bulk write from the client, no Keep).
//! - S7 over Link: output enables cut between two writes end reverted.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, GuardPhase};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "P1SA-LINK";
/// Every block of a document.
const ALL: u32 = dspi_core::consts::IMPORT_BY_BLOCK | dspi_core::consts::IMPORT_ALL_BLOCKS;

/// Bulk images written to the device: a `0xA1`, or a chunked one's first chunk.
#[derive(Clone, Default)]
struct Bulk(Arc<Mutex<usize>>);

impl Bulk {
    fn count(&self) -> usize {
        *self.0.lock().unwrap()
    }
}

struct Probe {
    inner: FakeDevice,
    bulk: Bulk,
    descriptor: DeviceDescriptor,
}

impl Transport for Probe {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        if o == op::REQ_SET_ALL_PARAMS || (o == op::REQ_SET_ALL_PARAMS_CHUNK && v == 0) {
            *self.bulk.0.lock().unwrap() += 1;
        }
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

struct Rig {
    _hub: Hub,
    url: String,
    remote: FakeRemote,
    bulk: Bulk,
}

/// A hub on its real server (open access, loopback) with one demo device.
fn rig() -> Rig {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p1sa-link").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process("P1SA hub")
    })
    .unwrap();
    let inner = FakeDevice::demo();
    let remote = inner.remote();
    let bulk = Bulk::default();
    let probe = Probe {
        inner,
        bulk: bulk.clone(),
        descriptor: DeviceDescriptor {
            serial: SERIAL.into(),
            bus_id: "test".into(),
            address: 1,
        },
    };
    hub.attach(Box::new(probe), 0).unwrap();
    let port = hub.network().unwrap().port();
    Rig {
        _hub: hub,
        url: format!("ws://127.0.0.1:{port}/dspi/v1"),
        remote,
        bulk,
    }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `core` on the hub, bound to the device and read.
fn connect_on(core: &DspiCore, url: &str) {
    core.link_connect(
        url.to_owned(),
        "wp-p1sa".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
}

fn connect(url: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    connect_on(&core, url);
    core
}

/// The hub's `apply.status` for the bound device.
fn hub_status(core: &DspiCore) -> Value {
    let handle = core.link_devices().remove(0).handle;
    let mut f = serde_json::Map::new();
    f.insert("handle".into(), json!(handle));
    core.link_hub_request("apply.status", f).unwrap()
}

/// A document with another preamp and output gain than the device has; the
/// device is left as it was.
fn changed_document(core: &DspiCore) -> String {
    let back = core
        .snapshot_capture_document("back".into(), String::new())
        .unwrap();
    core.set_input_preamp(0, -6.0).unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    let doc = core
        .config_export("Kväll".into(), "wp-p1sa".into(), 0)
        .unwrap();
    core.snapshot_recall_document(back).unwrap();
    doc
}

#[test]
fn wp_p1sa_link_confirm_keeps() {
    let r = rig();
    let core = connect(&r.url);
    assert!(core.apply_guard_default(), "guarded by default over Link");
    let doc = changed_document(&core);
    let before = r.remote.image();
    let s = core.apply_guarded_import(doc, ALL, u32::MAX, 0).unwrap();
    assert_eq!(s.phase, GuardPhase::Waiting);
    assert!(s.hub, "the hub guards it too");
    let st = hub_status(&core);
    assert!(
        st["active"]["remaining_ms"].as_u64().unwrap() > 30_000,
        "the hub's deadline is after the core's: {st}"
    );
    let applied = r.remote.image();
    assert_ne!(applied, before);
    let k = core.apply_confirm().unwrap();
    assert_eq!((k.phase, k.reason.as_str()), (GuardPhase::Kept, "confirmed"));
    let st = hub_status(&core);
    assert_eq!(st["active"], Value::Null, "{st}");
    assert_eq!(st["last"]["reason"], "confirmed");
    assert_eq!(st["last"]["result"], "kept");
    assert_eq!(r.remote.image(), applied);
}

#[test]
fn wp_p1sa_link_the_polling_client_reverts_at_its_deadline() {
    let r = rig();
    let core = connect(&r.url);
    let doc = changed_document(&core);
    let before = r.remote.image();
    let state = core.get_state();
    core.apply_guarded_import(doc, ALL, u32::MAX, 1).unwrap();
    assert_ne!(r.remote.image(), before);
    std::thread::sleep(Duration::from_millis(1100));
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "timeout"),
        "{end:?}"
    );
    assert_eq!(r.remote.image(), before);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state(), state);
    // The hub's guard ended with the core's revert, not at its own deadline.
    let st = hub_status(&core);
    assert_eq!(st["active"], Value::Null, "{st}");
    assert_eq!(st["last"]["reason"], "cancelled");
    assert_eq!(st["last"]["result"], "reverted");
}

#[test]
fn wp_p1sa_link_a_silent_client_is_reverted_by_the_hub() {
    let r = rig();
    let core = connect(&r.url);
    let doc = changed_document(&core);
    let before = r.remote.image();
    core.apply_guarded_import(doc, ALL, u32::MAX, 1).unwrap();
    assert_ne!(r.remote.image(), before);
    // No poll: the hub's own timer (the core's deadline plus its margin).
    wait_until("the hub's revert", || r.remote.image() == before);
    let writes = r.bulk.count();
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "timeout"),
        "{end:?}"
    );
    assert!(end.message.contains("hub"), "{}", end.message);
    assert_eq!(r.bulk.count(), writes, "the client wrote nothing more");
}

#[test]
fn wp_p1sa_link_a_cut_is_reverted_by_the_hub_and_said_on_reconnect() {
    let r = rig();
    let core = connect(&r.url);
    let doc = changed_document(&core);
    let before = r.remote.image();
    core.apply_guarded_import(doc, ALL, u32::MAX, 30).unwrap();
    assert_ne!(r.remote.image(), before);
    core.link_close();
    wait_until("the hub's revert", || r.remote.image() == before);
    let writes = r.bulk.count();
    // Lost until the hub can be asked; nothing is sent.
    let lost = core.apply_poll();
    assert_eq!(lost.phase, GuardPhase::Lost, "{lost:?}");
    assert_eq!(
        core.apply_confirm().unwrap().phase,
        GuardPhase::Lost,
        "a Keep is not queued"
    );
    connect_on(&core, &r.url);
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "connection_lost"),
        "{end:?}"
    );
    assert!(end.message.contains("hub put"), "{}", end.message);
    assert_eq!(core.apply_poll(), end);
    // Nothing replayed: no bulk write, no Keep reached the hub.
    assert_eq!(r.bulk.count(), writes);
    assert_eq!(r.remote.image(), before);
    let st = hub_status(&core);
    assert_eq!(st["last"]["reason"], "session_closed");
    assert_eq!(st["last"]["result"], "reverted");
    // The event log says so.
    let log = core.event_log_read(String::new(), 0, 1000).entries;
    assert!(
        log.iter().any(|e| e.text.starts_with("The hub put back")),
        "{log:?}"
    );
}

#[test]
fn wp_p1sa_link_s7_output_enables_cut_between_writes() {
    let r = rig();
    let core = connect(&r.url);
    let before = r.remote.image();
    let s = core.apply_guard_begin("Outputs off".into(), 30).unwrap();
    assert!(s.hub);
    core.set_output_enable(0, false).unwrap();
    assert_ne!(r.remote.image(), before);
    core.link_close();
    wait_until("the hub's revert", || r.remote.image() == before);
    connect_on(&core, &r.url);
    assert_eq!(core.apply_poll().phase, GuardPhase::Reverted);
    assert!(core.get_state().output_enabled[0]);
}
