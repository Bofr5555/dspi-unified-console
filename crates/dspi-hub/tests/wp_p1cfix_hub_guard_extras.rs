//! WP P1CFIX, F6 (review p1c): the hub's guarded-apply revert puts back the
//! settings outside the image too, through the real hub and the real client
//! (`DspiCore` over Link): the device and source names, the output latency and
//! the control-surface configuration, captured at `apply.begin`, written back
//! when the client vanishes, past another session's lock, once, only to the
//! same serial; the client says so after the reconnect.
//!
//! The rig is P1SA's (`wp_p1sa_guarded_link.rs`).
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
const SERIAL: &str = "P1CFIX-LINK";

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
    let dir = std::env::temp_dir().join("wp-p1cfix-link").join(format!(
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
        ..HubConfig::in_process("P1CFIX hub")
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
        "wp-p1cfix".into(),
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

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn latency(core: &DspiCore) -> u8 {
    let l = core.fetch_output_latency().unwrap();
    let f = l.fields.iter().find(|f| f.name == "requested_id").unwrap();
    f.value.parse().unwrap()
}

/// The control-surface configuration (not live readings).
fn surfaces(core: &DspiCore) -> Value {
    core.cs_fetch().unwrap();
    let mut cs = json_of(&core.cs_snapshot());
    if let Some(o) = cs.as_object_mut() {
        o.retain(|k, _| {
            matches!(
                k.as_str(),
                "bindings" | "names" | "ir" | "groups" | "macros" | "display"
            )
        });
    }
    if let Some(d) = cs.get_mut("display").and_then(Value::as_object_mut) {
        d.retain(|k, _| matches!(k.as_str(), "config" | "pages"));
    }
    cs
}

/// What the hub's revert must put back besides the image.
#[derive(Debug, PartialEq)]
struct Outside {
    device_name: String,
    source_names: Vec<String>,
    latency: u8,
    surfaces: Value,
}

fn outside(core: &DspiCore) -> Outside {
    core.fetch_all().unwrap();
    Outside {
        device_name: core.get_device_name().unwrap(),
        source_names: (0..2)
            .map(|s| core.get_input_source_name(s).unwrap())
            .collect(),
        latency: latency(core),
        surfaces: surfaces(core),
    }
}

fn cs_ok(core: &DspiCore, op: Value) {
    let r = json_of(&core.cs_apply(op.to_string()));
    assert_eq!(r["ok"], true, "{op}: {r}");
}

/// Change every part outside the image (and one value inside it).
fn change_outside(core: &DspiCore) {
    core.set_output_gain(1, -3.0).unwrap();
    core.set_device_name("Changed".into()).unwrap();
    core.set_input_source_name(1, "Other".into()).unwrap();
    let was = latency(core);
    core.set_output_latency(if was == 3 { 4 } else { 3 })
        .unwrap();
    let s = json_of(&core.cs_snapshot());
    let mut g = s["groups"][0].clone();
    g["name"] = json!("Moved group");
    cs_ok(core, json!({ "op": "group", "index": 0, "group": g }));
    cs_ok(
        core,
        json!({ "op": "name", "index": 0, "name": "Renamed knob" }),
    );
}

/// A client that vanishes mid-apply: the hub puts back the image and every part
/// outside it; the outcome lists what it held and what it wrote; the client
/// says so after the reconnect, and nothing is replayed.
#[test]
fn wp_p1cfix_the_hub_puts_back_the_settings_outside_the_image() {
    let r = rig();
    let core = connect(&r.url);
    core.cs_fetch().unwrap();
    let image = r.remote.image();
    let was = outside(&core);
    let s = core.apply_guard_begin("Installer flow".into(), 30).unwrap();
    assert!(s.hub);
    change_outside(&core);
    assert_ne!(outside(&core), was, "the flow changed every part");
    core.link_close();
    wait_until("the hub's revert", || r.remote.image() == image);
    let writes = r.bulk.count();
    connect_on(&core, &r.url);
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "connection_lost"),
        "{end:?}"
    );
    assert_eq!(outside(&core), was, "every part outside the image is back");
    assert_eq!(r.remote.image(), image);
    assert_eq!(r.bulk.count(), writes, "nothing replayed");
    // The outcome: what the hub held and what it wrote back.
    let st = hub_status(&core);
    let parts = |k: &str| -> Vec<String> {
        st["last"][k]
            .as_array()
            .unwrap_or_else(|| panic!("{k}: {st}"))
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    let all = [
        "device_name",
        "source_names",
        "output_latency",
        "control_surfaces",
    ];
    assert_eq!(parts("captured"), all, "{st}");
    assert_eq!(parts("restored"), all, "{st}");
    assert!(st["last"].get("notes").is_none(), "nothing refused: {st}");
    // The client's words no longer say "image only".
    assert!(
        end.notes.iter().all(|n| !n.contains("image only")),
        "{:?}",
        end.notes
    );
}

/// Nothing changed outside the image: the hub writes none of it (only what
/// differs is written), and the outcome says what it held.
#[test]
fn wp_p1cfix_the_hub_writes_only_what_differs() {
    let r = rig();
    let core = connect(&r.url);
    let image = r.remote.image();
    let was = outside(&core);
    core.apply_guard_begin("Gain only".into(), 30).unwrap();
    core.set_output_gain(1, -3.0).unwrap();
    core.link_close();
    wait_until("the hub's revert", || r.remote.image() == image);
    connect_on(&core, &r.url);
    assert_eq!(core.apply_poll().phase, GuardPhase::Reverted);
    assert_eq!(outside(&core), was);
    let st = hub_status(&core);
    assert_eq!(
        st["last"]["captured"].as_array().map(Vec::len),
        Some(4),
        "{st}"
    );
    assert!(
        st["last"].get("restored").is_none(),
        "nothing differed: {st}"
    );
}

/// A silent client that holds the device's lock is reverted by the hub's timer
/// past that lock, the settings outside the image included (reverts are never
/// refused by a lock: the router's revert window).
#[test]
fn wp_p1cfix_the_hub_reverts_past_the_lock_of_a_silent_client() {
    let r = rig();
    let core = connect(&r.url);
    core.cs_fetch().unwrap();
    let image = r.remote.image();
    let was = outside(&core);
    core.device_lock_acquire("wp-p1cfix".into(), 60_000)
        .unwrap();
    core.apply_guard_begin("Locked flow".into(), 1).unwrap();
    change_outside(&core);
    // No poll: the hub's own timer.
    wait_until("the hub's revert", || {
        hub_status(&core)["last"]["result"] == "reverted"
    });
    assert_eq!(r.remote.image(), image);
    let end = core.apply_poll();
    assert_eq!(end.phase, GuardPhase::Reverted, "{end:?}");
    assert_eq!(outside(&core), was, "written back past the lock");
    let st = hub_status(&core);
    assert!(st["last"].get("notes").is_none(), "nothing refused: {st}");
    core.device_lock_release();
}
