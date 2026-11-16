//! WP P3GP's done criteria through the real server with two demo devices: linked
//! parameters (`group.param.get`, `group.param.set` with offsets, `group.preset.load`
//! with a map; `hub-api.md` section 20.2) over a WebSocket of a paired client with its
//! role, each member run as the caller's child session and judged on its own.
#![cfg(feature = "api")]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_hub::{
    AccessMode, HistoryConfig, Hub, HubConfig, NetworkConfig, Role, ServiceConfig, TestClock,
};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, NotificationSource, Result as TResult, Transport, TransportCaps,
};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
/// Monday 2027-01-04 12:00:00 UTC.
const MONDAY_NOON: u64 = 1_799_064_000;
const A: &str = "P3GP-A";
const B: &str = "P3GP-B";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3gp").join(format!(
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
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Rig {
    hub: Hub,
}

fn start(dir: &Path, groups: bool) -> Hub {
    let clock = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(MONDAY_NOON)));
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            clock: clock as Arc<dyn dspi_hub::HubClock>,
            inventory: true,
            groups,
            history: Some(HistoryConfig::default()),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p3gp")
    })
    .unwrap()
}

fn rig(tag: &str) -> Rig {
    Rig {
        hub: start(&scratch(tag), true),
    }
}

impl Rig {
    fn plug(&self, serial: &str) -> u8 {
        self.hub
            .attach(Box::new(FakeDevice::demo().with_serial(serial)), 0)
            .unwrap()
    }

    /// A paired client with `role`, logged in over the socket.
    fn client(&self, name: &str, role: Role) -> Ws {
        let net = self.hub.network().unwrap();
        let window = net.auth().begin_pairing(Instant::now()).unwrap();
        let authed = net
            .auth()
            .pair(
                &window.pin,
                name,
                Some(role.min(Role::Control)),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                Instant::now(),
            )
            .unwrap();
        if role == Role::Admin {
            assert!(net.auth().set_role(authed.cid, Role::Admin));
        }
        let mut ws = Ws::connect(net.port());
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": name, "app": "wp_p3gp", "version": "0"}}));
        let hello = ws.next("hello");
        ws.caps = hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws
    }

    /// Two demo devices in group `g1` ([`A`], [`B`]), made by an Admin.
    fn two_in_a_group(&self) -> (u8, u8, Ws) {
        let ha = self.plug(A);
        let hb = self.plug(B);
        let mut admin = self.client("Installer", Role::Admin);
        // The inventory has them once it has seen them online.
        let start = Instant::now();
        loop {
            let v = admin.ask(json!({"t": "group.set",
                "group": {"name": "Hall", "members": [A, B]}}));
            if ok(&v) {
                assert_eq!(v["group"]["id"], "g1", "{v}");
                break;
            }
            assert!(start.elapsed() < WAIT, "{v}");
            std::thread::sleep(Duration::from_millis(20));
        }
        (ha, hb, admin)
    }
}

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    caps: Vec<String>,
}

impl Ws {
    fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        Self {
            ws: tungstenite::client(req, tcp).unwrap().0,
            next_id: 100,
            caps: Vec::new(),
        }
    }

    fn send(&mut self, v: Value) {
        self.ws.send(Message::text(v.to_string())).unwrap();
    }

    fn read(&mut self) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no text frame in time");
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn next(&mut self, t: &str) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            let v = self.read();
            if v["t"] == t {
                return v;
            }
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.send(msg);
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            let v = self.read();
            if v["id"] == id && (v["t"] == "ok" || v["t"] == "err") {
                return v;
            }
        }
    }

    fn get(&mut self, serial: &str, path: &str) -> Value {
        let v = self.ask(json!({"t": "param.get", "serial": serial, "path": path}));
        assert!(ok(&v), "{v}");
        v["value"].clone()
    }

    fn records(&mut self, serial: &str) -> Vec<Value> {
        let list = self.ask(json!({"t": "event.list", "serial": serial, "limit": 500}));
        assert!(ok(&list), "{list}");
        list["records"].as_array().unwrap().clone()
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

/// The row of `serial` in an answer.
fn row<'a>(v: &'a Value, serial: &str) -> &'a Value {
    v["results"]
        .as_array()
        .unwrap_or_else(|| panic!("no results: {v}"))
        .iter()
        .find(|r| r["serial"] == serial)
        .unwrap_or_else(|| panic!("no row {serial}: {v}"))
}

fn serials(v: &Value) -> Vec<String> {
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["serial"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn wp_p3gp_a_volume_set_with_offsets_lands_on_both_clamped_and_attributed() {
    let r = rig("set");
    let (_, _, _admin) = r.two_in_a_group();
    let mut c = r.client("Front of house", Role::Control);
    assert!(c.caps.iter().any(|x| x == "group_params"), "{:?}", c.caps);

    // -20 dB, B 3 dB lower.
    let v = c.ask(
        json!({"t": "group.param.set", "group": "g1", "path": "vol.master",
        "value": -20.0, "offsets": {B: -3.0}}),
    );
    assert!(ok(&v), "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(
        (v["path"].as_str(), &v["idx"]),
        (Some("vol.master"), &json!([]))
    );
    assert_eq!(serials(&v), [A, B], "group order");
    assert_eq!(row(&v, A)["ok"], true, "{v}");
    assert_eq!(row(&v, A)["value"], -20.0, "{v}");
    assert_eq!(row(&v, B)["value"], -23.0, "{v}");
    assert!(row(&v, B).get("clamped").is_none(), "{v}");
    assert_eq!(c.get(A, "vol.master"), -20.0);
    assert_eq!(c.get(B, "vol.master"), -23.0);

    // A 5 dB above -2 is past the device's 0 dB: clamped there, and the row says so.
    let v = c.ask(
        json!({"t": "group.param.set", "group": "g1", "path": "vol.master",
        "value": -2.0, "offsets": {A: 5.0}}),
    );
    assert!(ok(&v), "{v}");
    assert_eq!(row(&v, A)["value"], 0.0, "{v}");
    assert_eq!(row(&v, A)["clamped"], true, "{v}");
    assert_eq!(row(&v, B)["value"], -2.0, "{v}");
    assert!(row(&v, B).get("clamped").is_none(), "{v}");

    // group.param.get returns both values.
    let g = c.ask(json!({"t": "group.param.get", "group": "g1", "path": "vol.master"}));
    assert!(ok(&g), "{g}");
    assert_eq!(serials(&g), [A, B]);
    assert_eq!(
        (row(&g, A)["value"].as_f64(), row(&g, B)["value"].as_f64()),
        (Some(0.0), Some(-2.0))
    );

    // The history of each device has the write, with the client's name.
    for serial in [A, B] {
        wait_until("an attributed record", || {
            c.records(serial)
                .iter()
                .any(|rec| rec["kind"] == "notification" && rec["by"] == "Front of house")
        });
    }
    r.hub.stop();
}

#[test]
fn wp_p3gp_a_viewer_reads_and_is_refused_writes() {
    let r = rig("viewer");
    r.two_in_a_group();
    let mut v = r.client("Back office", Role::Viewer);
    assert!(v.caps.iter().any(|x| x == "group_params"));
    let before = v.get(A, "vol.master");
    let set = v.ask(
        json!({"t": "group.param.set", "group": "g1", "path": "vol.master",
        "value": -30.0}),
    );
    assert_eq!(code(&set), "denied", "{set}");
    assert!(set.get("results").is_none(), "nothing ran: {set}");
    let load = v.ask(json!({"t": "group.preset.load", "group": "g1", "slot": 0}));
    assert_eq!(code(&load), "denied", "{load}");
    assert_eq!(v.get(A, "vol.master"), before, "nothing reached the device");
    let get = v.ask(json!({"t": "group.param.get", "targets": [A, B], "path": "vol.master"}));
    assert!(ok(&get), "{get}");
    assert_eq!(row(&get, A)["value"], before);
    r.hub.stop();
}

#[test]
fn wp_p3gp_a_control_row_past_a_limit_is_limit_exceeded_and_the_other_is_written() {
    let r = rig("limit");
    let (ha, _, mut admin) = r.two_in_a_group();
    let set = admin.ask(json!({"t": "limits.set", "handle": ha,
        "profile": {"masterVolumeMaxDb": -10.0}}));
    assert!(ok(&set), "{set}");
    let mut c = r.client("Venue tablet", Role::Control);
    let before = c.get(A, "vol.master");

    let v = c.ask(
        json!({"t": "group.param.set", "group": "g1", "path": "vol.master",
        "value": -5.0}),
    );
    assert!(ok(&v), "{v}");
    let a = row(&v, A);
    assert_eq!(
        (a["ok"].as_bool(), code(a), a["limit"].as_str()),
        (Some(false), "limit_exceeded", Some("master_volume")),
        "{v}"
    );
    assert_eq!(row(&v, B)["value"], -5.0, "{v}");
    assert_eq!(c.get(A, "vol.master"), before, "A got nothing");
    assert_eq!(c.get(B, "vol.master"), -5.0);
    r.hub.stop();
}

#[test]
fn wp_p3gp_an_offline_member_is_offline_at_once_and_unknown_ones_not_found() {
    let r = rig("offline");
    let (_, hb, _admin) = r.two_in_a_group();
    let mut c = r.client("Front of house", Role::Control);
    r.hub.handle().detach(hb).unwrap();
    wait_until("B offline", || {
        r.hub.devices().iter().any(|d| {
            d.identity.serial == B && d.handle == hb && d.state != dspi_hub::DeviceState::Online
        }) || !r.hub.devices().iter().any(|d| d.identity.serial == B)
    });

    let started = Instant::now();
    let v = c.ask(
        json!({"t": "group.param.set", "group": "g1", "path": "vol.master",
        "value": -12.0}),
    );
    assert!(started.elapsed() < Duration::from_secs(5), "nothing waited");
    assert!(ok(&v), "{v}");
    assert_eq!(row(&v, A)["value"], -12.0, "{v}");
    assert_eq!(code(row(&v, B)), "offline", "{v}");
    // By targets: a serial the hub knows is offline, one it never heard of not_found.
    let v = c.ask(
        json!({"t": "group.param.get", "targets": ["P3GP-NEVER", B, A],
        "path": "vol.master"}),
    );
    assert!(ok(&v), "{v}");
    assert_eq!(serials(&v), ["P3GP-NEVER", B, A], "request order");
    assert_eq!(code(row(&v, "P3GP-NEVER")), "not_found", "{v}");
    assert_eq!(code(row(&v, B)), "offline", "{v}");
    assert_eq!(row(&v, A)["value"], -12.0, "{v}");
    r.hub.stop();
}

#[test]
fn wp_p3gp_malformed_requests_are_refused_whole() {
    let r = rig("bad");
    r.two_in_a_group();
    let mut c = r.client("Front of house", Role::Control);
    let before = c.get(A, "vol.master");
    for (msg, want) in [
        // An offset on a path that is not in dB.
        (
            json!({"t": "group.param.set", "group": "g1", "path": "vol.mute", "value": true,
                "offsets": {A: 1.0}}),
            "bad_request",
        ),
        // An offset for a device the request does not address.
        (
            json!({"t": "group.param.set", "targets": [A], "path": "vol.master", "value": -9.0,
                "offsets": {B: -3.0}}),
            "bad_request",
        ),
        // Outside the registry's range.
        (
            json!({"t": "group.param.set", "group": "g1", "path": "vol.master", "value": 6.0}),
            "bad_request",
        ),
        // A gated path is param.set's alone.
        (
            json!({"t": "group.param.set", "group": "g1", "path": "dev.save", "value": null}),
            "bad_request",
        ),
        // Both ways of naming devices, or neither.
        (
            json!({"t": "group.param.set", "group": "g1", "targets": [A], "path": "vol.master",
                "value": -9.0}),
            "bad_request",
        ),
        (
            json!({"t": "group.param.get", "path": "vol.master"}),
            "bad_request",
        ),
        // A group the hub does not have.
        (
            json!({"t": "group.param.set", "group": "g9", "path": "vol.master", "value": -9.0}),
            "not_found",
        ),
        // A map entry that is no slot.
        (
            json!({"t": "group.preset.load", "group": "g1", "slot": 1, "map": {B: 12}}),
            "bad_request",
        ),
    ] {
        let v = c.ask(msg.clone());
        assert_eq!(code(&v), want, "{msg} -> {v}");
        assert!(v.get("results").is_none(), "{v}");
    }
    assert_eq!(c.get(A, "vol.master"), before, "nothing was written");
    r.hub.stop();
}

#[test]
fn wp_p3gp_a_preset_map_loads_slot_3_here_and_5_there() {
    let r = rig("presets");
    let (_, _, mut admin) = r.two_in_a_group();
    for (serial, slot, name) in [(A, 3, "Show A"), (B, 5, "Show B"), (B, 3, "Spare")] {
        let v = admin.ask(json!({"t": "preset.save", "serial": serial, "slot": slot,
            "name": name, "confirm": true}));
        assert!(ok(&v), "{v}");
    }
    let mut c = r.client("Front of house", Role::Control);
    let v = c.ask(json!({"t": "group.preset.load", "group": "g1", "slot": 3, "map": {B: 5}}));
    assert!(ok(&v), "{v}");
    assert_eq!(serials(&v), [A, B]);
    assert_eq!(
        (row(&v, A)["slot"].as_u64(), row(&v, A)["name"].as_str()),
        (Some(3), Some("Show A")),
        "{v}"
    );
    assert_eq!(
        (row(&v, B)["slot"].as_u64(), row(&v, B)["name"].as_str()),
        (Some(5), Some("Show B")),
        "{v}"
    );
    // By name, the map by name too.
    let v = c.ask(
        json!({"t": "group.preset.load", "targets": [A, B], "name": "Show A",
        "map": {B: "Show B"}}),
    );
    assert!(ok(&v), "{v}");
    assert_eq!(row(&v, A)["slot"], 3, "{v}");
    assert_eq!(row(&v, B)["slot"], 5, "{v}");
    // A name a member does not have is that row's not_found.
    let v = c.ask(json!({"t": "group.preset.load", "group": "g1", "name": "Show A"}));
    assert!(ok(&v), "{v}");
    assert_eq!(row(&v, A)["slot"], 3, "{v}");
    assert_eq!(code(row(&v, B)), "not_found", "{v}");
    r.hub.stop();
}

#[test]
fn wp_p3gp_without_groups_a_group_is_not_found_and_targets_work() {
    let r = Rig {
        hub: start(&scratch("nogroups"), false),
    };
    r.plug(A);
    let mut c = r.client("Front of house", Role::Control);
    let v = c.ask(json!({"t": "group.param.get", "group": "g1", "path": "vol.master"}));
    assert_eq!(code(&v), "not_found", "{v}");
    let v = c.ask(
        json!({"t": "group.param.set", "targets": [A], "path": "vol.master",
        "value": -7.0}),
    );
    assert_eq!(row(&v, A)["value"], -7.0, "{v}");
    r.hub.stop();
}

// ── One run at a time per session ───────────────────────────────────────────

/// The demo device, holding a master-volume write while `held` is set.
struct Gated {
    inner: FakeDevice,
    held: Arc<AtomicBool>,
    waiting: Arc<AtomicBool>,
}

impl Transport for Gated {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.inner.control_in(opcode, value, len)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        if opcode == op::REQ_SET_MASTER_VOLUME {
            let start = Instant::now();
            while self.held.load(Ordering::SeqCst) && start.elapsed() < WAIT {
                self.waiting.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
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

#[test]
fn wp_p3gp_a_second_run_of_the_same_session_while_one_runs_is_busy() {
    let r = rig("busy");
    let held = Arc::new(AtomicBool::new(true));
    let waiting = Arc::new(AtomicBool::new(false));
    r.hub
        .attach(
            Box::new(Gated {
                inner: FakeDevice::demo().with_serial(A),
                held: Arc::clone(&held),
                waiting: Arc::clone(&waiting),
            }),
            0,
        )
        .unwrap();
    r.plug(B);
    let h = r.hub.handle();
    let key = h.open_principal(Role::Control, None, "Venue key").unwrap();
    let other = h.open_principal(Role::Control, None, "Other key").unwrap();
    let ask = |s: &Arc<dspi_hub::session::Session>, msg: Value| -> Value {
        serde_json::from_str(&h.service_request(s, &msg).unwrap()).unwrap()
    };

    let first = {
        let h = h.clone();
        let key = Arc::clone(&key);
        std::thread::spawn(move || {
            let msg = json!({"t": "group.param.set", "id": 1, "targets": [A],
                "path": "vol.master", "value": -31.0});
            let text = h.service_request(&key, &msg).unwrap();
            serde_json::from_str::<Value>(&text).unwrap()
        })
    };
    wait_until("the first run at A's write", || {
        waiting.load(Ordering::SeqCst)
    });
    let second = ask(
        &key,
        json!({"t": "group.param.get", "id": 2, "targets": [B], "path": "vol.master"}),
    );
    assert_eq!(code(&second), "busy", "{second}");
    // Another session is not held up.
    let elsewhere = ask(
        &other,
        json!({"t": "group.param.get", "id": 3, "targets": [B], "path": "vol.master"}),
    );
    assert!(ok(&elsewhere), "{elsewhere}");

    held.store(false, Ordering::SeqCst);
    let first = first.join().unwrap();
    assert!(ok(&first), "{first}");
    assert_eq!(row(&first, A)["value"], -31.0, "{first}");
    // Done: the session runs again.
    let again = ask(
        &key,
        json!({"t": "group.param.get", "id": 4, "targets": [A, B], "path": "vol.master"}),
    );
    assert!(ok(&again), "{again}");
    assert_eq!(row(&again, A)["value"], -31.0, "{again}");
    h.close_session(key.id());
    h.close_session(other.id());
    r.hub.stop();
}
