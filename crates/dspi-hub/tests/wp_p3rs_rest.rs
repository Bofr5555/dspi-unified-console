//! WP P3RS's done criteria through the real server: the fleet's REST endpoints
//! (`hub-api.md` section 14.2, PLAN-P3 OD-P3l) on two demo devices in a group, each
//! request over TLS with an API key as its principal, on a hub in `--tls on` and in
//! `--tls require` (each test runs both, whatever `DSPI_HUB_TEST_TLS` says): each
//! endpoint onto its message, a Viewer key's `PUT` `403`, a guarded group apply confirmed
//! by the same key and reverted when another key tries, a rollout `POST` `405` with
//! `Allow: GET`, and the rollout's reads refused over plaintext (`426` `tls_required`)
//! with a `plaintext_ok` key and in upstream compatibility.
//!
//! No USB: the rollout service runs with a BOOTSEL scan that finds nothing and a writer
//! that refuses; nothing here starts a rollout (no REST path can). Its record and image
//! are put on disk before the hub starts.
#![cfg(feature = "rollout")]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{BootDevice, DspiCore, FirmwareGuardBoard, FlashReport};
use dspi_hub::rollout::{BootScan, RolloutConfig, Writer};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role, ServiceConfig, TlsMode};
use dspi_link::tls::{self as link_tls, PinMode, TlsPipe};
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(30);
const A: &str = "P3RS-A";
const B: &str = "P3RS-B";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3rs").join(format!(
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
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ── An image for the store (laid out as the firmware guard's tests build them) ──

const XIP: u32 = 0x1000_0000;
const RP2350: u32 = 0xE48B_FF59;

fn image(version: &str, stamp: &str) -> Vec<u8> {
    let entries: Vec<(u16, u32, &str)> = vec![
        (0x5052, 0x0203_1C86, "DSPi"),
        (0x5052, 0x11A9_BC3A, version),
        (0x5344, 0x0B7A_4C15, stamp),
    ];
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, 0x7188_EBF2);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, 0xE71A_A390);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&6u16.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let mut b = vec![0u8; 512];
        for (off, v) in [
            (0, 0x0A32_4655u32),
            (4, 0x9E5D_5157),
            (8, 0x2000),
            (12, XIP + 256 * i as u32),
            (16, 256),
            (20, i as u32),
            (24, pages.len() as u32),
            (28, RP2350),
            (508, 0x0AB1_6F30),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..32 + p.len()].copy_from_slice(p);
        out.extend(b);
    }
    out
}

/// No board is ever in BOOTSEL, and nothing is ever written.
struct NoBoards;

impl BootScan for NoBoards {
    fn boards(&self) -> Vec<BootDevice> {
        Vec::new()
    }
}

impl Writer for NoBoards {
    fn flash(
        &self,
        _image: &[u8],
        _target: &BootDevice,
        _board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        Err("no writer in this test".into())
    }
}

// ── The hub ─────────────────────────────────────────────────────────────────

struct Rig {
    hub: Hub,
    dir: PathBuf,
    port: u16,
}

/// A hub in `tls` (upstream compatibility with `compat`) with REST, the inventory,
/// groups, the snapshot store and the rollout; one finished rollout `ro1` and one image
/// on disk; `A` and `B` plugged.
fn rig(tag: &str, tls: TlsMode, compat: bool) -> Rig {
    let dir = scratch(tag);
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();
    dspi_hub::rollout::images::add_file(
        &store,
        "dspi-1.1.7.uf2",
        &image("1.1.7", "1200"),
        1_800_000_000,
    )
    .unwrap();
    let record = json!({"v": 1, "next_id": 2, "rollouts": [{
        "id": "ro1", "target": A,
        "image": {"name": "dspi-1.1.7.uf2", "sha256": "00", "firmware_version": "1.1.7"},
        "by": "Bench console", "while_live": false, "state": "ok",
        "started_at": "2027-01-15T08:00:00Z", "ended_at": "2027-01-15T08:01:00Z",
        "devices": []}]});
    std::fs::write(store.join("rollout.json"), record.to_string()).unwrap();
    let fakes = Arc::new(NoBoards);
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            rest: true,
            tls,
            upstream_compat: compat,
            ..NetworkConfig::new(store)
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            inventory: true,
            groups: true,
            rollout: Some(RolloutConfig {
                allow_unverified_firmware: true,
                boot_scan: Some(Arc::clone(&fakes) as Arc<dyn BootScan>),
                writer: Some(fakes as Arc<dyn Writer>),
                serves_tls: true,
            }),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p3rs")
    })
    .unwrap();
    let port = hub.network().unwrap().port();
    let r = Rig { hub, dir, port };
    for s in [A, B] {
        r.hub
            .attach(Box::new(FakeDevice::demo().with_serial(s)), 0)
            .unwrap();
    }
    r
}

fn raw(port: u16) -> TcpStream {
    let tcp = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp.set_write_timeout(Some(WAIT)).unwrap();
    tcp
}

impl Rig {
    fn tls(&self) -> TlsPipe {
        let pin = self
            .hub
            .network()
            .unwrap()
            .tls()
            .pin
            .expect("TLS is served");
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut pipe, _) =
            link_tls::connect(raw(self.port), "localhost", PinMode::Pinned(pin), deadline).unwrap();
        pipe.set_deadline(None);
        pipe
    }

    /// An API key minted on the hub's machine.
    fn key(&self, name: &str, role: Role, plaintext_ok: bool) -> String {
        self.hub
            .network()
            .unwrap()
            .auth()
            .mint_api_key(name, role, plaintext_ok)
            .unwrap()
            .key
    }

    /// A REST request over TLS.
    fn call(&self, method: &str, path: &str, key: &str, body: Option<Value>) -> Answer {
        http(self.tls(), self.port, method, path, key, body)
    }

    /// The same over plaintext.
    fn call_plain(&self, method: &str, path: &str, key: &str) -> Answer {
        http(raw(self.port), self.port, method, path, key, None)
    }

    /// Output 1's gain on `serial`, read with `key`.
    fn gain(&self, key: &str, serial: &str) -> f64 {
        let a = self.call(
            "GET",
            &format!("/api/v1/devices/{serial}/params/out.gain?idx=0"),
            key,
            None,
        );
        assert_eq!(a.status, 200, "{}", a.body);
        a.body["value"].as_f64().unwrap()
    }

    fn set_gain(&self, key: &str, serial: &str, gain: f64) {
        let a = self.call(
            "PUT",
            &format!("/api/v1/devices/{serial}/params/out.gain?idx=0"),
            key,
            Some(json!({"value": gain})),
        );
        assert_eq!(a.status, 200, "{}", a.body);
    }

    /// A paired Admin console over TLS.
    fn admin(&self) -> Ws {
        let net = self.hub.network().unwrap();
        let window = net.auth().begin_pairing(Instant::now()).unwrap();
        let authed = net
            .auth()
            .pair(
                &window.pin,
                "Installer",
                Some(Role::Control),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                Instant::now(),
            )
            .unwrap();
        assert!(net.auth().set_role(authed.cid, Role::Admin));
        let mut ws = Ws {
            ws: upgrade(self.tls(), self.port),
            next_id: 100,
        };
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "Installer", "app": "wp_p3rs", "version": "0"}}));
        let hello = ws.read_until(|v| v["t"] == "hello");
        assert!(hello["caps"].as_array().is_some(), "{hello}");
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws
    }

    /// Group `g1` of `A` and `B`; `A`'s state with output 1 at `gain` filed in the hub's
    /// store as "Quiet": its id.
    fn group_and_snapshot(&self, key: &str, gain: f64) -> (String, String) {
        let mut admin = self.admin();
        let g = admin.ask(json!({"t": "group.set", "group": {"name": "Hall", "members": [A, B]}}));
        assert_eq!(g["t"], "ok", "{g}");
        let group = g["group"]["id"].as_str().unwrap().to_owned();
        self.set_gain(key, A, gain);
        let core = DspiCore::new();
        let t = self.hub.handle().open_local(A).unwrap();
        core.open_transport(Box::new(t), A.into(), 0).unwrap();
        wait_until("the mirror", || {
            core.process_notifications();
            core.get_state().output_gain_db[0] == gain as f32
        });
        let _ = core.feature_present("device_name");
        let text = core
            .snapshot_capture_document("Quiet".into(), String::new())
            .unwrap();
        core.disconnect();
        let store = SnapshotStore::new(self.dir.join("client"));
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        static SAVED: AtomicI64 = AtomicI64::new(0);
        let at = 1_763_200_000_000 + SAVED.fetch_add(60_000, Ordering::SeqCst);
        let e = store.save(A, &mut doc, at).unwrap();
        let (_, filed) = store.read(&e.id).unwrap();
        let compact: Value = serde_json::from_str(&filed).unwrap();
        let compact = compact.to_string();
        if compact.len() < 60_000 {
            let put = admin.ask(json!({"t": "snapshot.put", "serial": A, "document": compact}));
            assert_eq!(put["t"], "ok", "{put}");
        } else {
            dspi_hub::snapshots::HubSnapshots::open(&self.dir.join("store"))
                .put(A, &filed)
                .unwrap();
        }
        (group, e.id)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.hub.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ── HTTP and WebSocket by hand ──────────────────────────────────────────────

struct Answer {
    status: u16,
    head: String,
    body: Value,
}

impl Answer {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    }

    fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or("")
    }

    /// `(serial, ok)` of each row.
    fn rows(&self) -> Vec<(String, bool)> {
        self.body["results"]
            .as_array()
            .unwrap_or_else(|| panic!("no results: {}", self.body))
            .iter()
            .map(|r| {
                (
                    r["serial"].as_str().unwrap().to_owned(),
                    r["ok"].as_bool().unwrap(),
                )
            })
            .collect()
    }
}

/// One request with `Connection: close` on `s`, and its answer.
fn http<S: Read + Write>(
    mut s: S,
    port: u16,
    method: &str,
    path: &str,
    key: &str,
    body: Option<Value>,
) -> Answer {
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Authorization: Bearer {key}\r\n"
    );
    let body = body.map(|b| b.to_string());
    if let Some(b) = &body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    } else if method == "PUT" {
        req.push_str("Content-Length: 0\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body.as_deref().unwrap_or(""));
    s.write_all(req.as_bytes()).unwrap();
    s.flush().unwrap();
    let mut all = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => all.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Answer {
        status: head
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0),
        head: head.to_owned(),
        body: serde_json::from_str(body).unwrap_or(Value::String(body.to_owned())),
    }
}

fn upgrade<S: Read + Write>(mut s: S, port: u16) -> WebSocket<S> {
    let req = format!(
        "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
    );
    s.write_all(req.as_bytes()).unwrap();
    s.flush().unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => head.push(byte[0]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    WebSocket::from_raw_socket(s, WsRole::Client, None)
}

struct Ws {
    ws: WebSocket<TlsPipe>,
    next_id: u64,
}

impl Ws {
    fn send(&mut self, v: Value) {
        self.ws.send(Message::text(v.to_string())).unwrap();
    }

    fn read_until(&mut self, pick: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "nothing in time");
            if let Message::Text(text) = self.ws.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if pick(&v) {
                    return v;
                }
            }
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.send(msg);
        self.read_until(|v| v["id"] == id && (v["t"] == "ok" || v["t"] == "err"))
    }
}

/// Both modes a fleet test runs in.
const MODES: [(TlsMode, &str); 2] = [(TlsMode::On, "on"), (TlsMode::Require, "require")];

// ── The tests ───────────────────────────────────────────────────────────────

#[test]
fn wp_p3rs_each_fleet_endpoint_answers_through_its_message() {
    for (mode, name) in MODES {
        let r = rig(&format!("each-{name}"), mode, false);
        let control = r.key("Venue script", Role::Control, false);
        let (g, snapshot) = r.group_and_snapshot(&control, -6.0);
        assert_eq!(g, "g1");

        // group.list, all and one
        let a = r.call("GET", "/api/v1/groups", &control, None);
        assert_eq!(a.status, 200, "{name}: {}", a.body);
        assert_eq!(a.body["groups"][0]["id"], "g1", "{}", a.body);
        assert_eq!(a.body["v"], 1);
        assert!(a.body["hub"].is_string() && a.body.get("t").is_none());
        let a = r.call("GET", "/api/v1/groups/g1", &control, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.body["groups"][0]["members"], json!([A, B]));
        let a = r.call("GET", "/api/v1/groups/g9", &control, None);
        assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);

        // group.param.set with an offset: B gets value + its delta
        let a = r.call(
            "PUT",
            "/api/v1/groups/g1/params/out.gain?idx=0",
            &control,
            Some(json!({"value": -3.0, "offsets": {B: -2.0}})),
        );
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.rows(), vec![(A.into(), true), (B.into(), true)]);
        assert_eq!(r.gain(&control, A), -3.0);
        assert_eq!(r.gain(&control, B), -5.0);
        // An unknown group's write
        let a = r.call(
            "PUT",
            "/api/v1/groups/g9/params/out.gain?idx=0",
            &control,
            Some(json!({"value": -3.0})),
        );
        assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);
        // GET is not group.param.get here: the path takes PUT only
        let a = r.call("GET", "/api/v1/groups/g1/params/out.gain", &control, None);
        assert_eq!(a.status, 405, "{}", a.body);
        assert_eq!(a.header("Allow").as_deref(), Some("PUT"));

        // group.preset.load: one row per member
        let a = r.call("POST", "/api/v1/groups/g1/presets/1/load", &control, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.rows().len(), 2, "{}", a.body);
        let a = r.call("POST", "/api/v1/groups/g1/presets/x/load", &control, None);
        assert_eq!((a.status, a.code()), (400, "bad_request"), "{}", a.body);

        // group.apply without a guard: both take A's -6
        r.set_gain(&control, A, -1.0);
        r.set_gain(&control, B, -2.0);
        let a = r.call(
            "POST",
            "/api/v1/groups/g1/apply",
            &control,
            Some(json!({"snapshot": snapshot})),
        );
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.rows(), vec![(A.into(), true), (B.into(), true)]);
        assert!(a.body["run"].is_string(), "{}", a.body);
        assert_eq!(r.gain(&control, A), -6.0);
        assert_eq!(r.gain(&control, B), -6.0);
        let run = a.body["run"].as_str().unwrap().to_owned();
        // A run without a guard confirms nothing
        let a = r.call(
            "POST",
            &format!("/api/v1/runs/{run}/confirm"),
            &control,
            None,
        );
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.body["results"], json!([]));
        let a = r.call("POST", "/api/v1/runs/a999/cancel", &control, None);
        assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);

        // The rollout's reads, over TLS
        let a = r.call("GET", "/api/v1/rollouts", &control, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.body["rollouts"][0]["id"], "ro1");
        let a = r.call("GET", "/api/v1/rollouts/ro1", &control, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.body["rollouts"][0]["by"], "Bench console");
        let a = r.call("GET", "/api/v1/rollouts/ro9", &control, None);
        assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);
        let a = r.call("GET", "/api/v1/firmware", &control, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.body["images"][0]["name"], "dspi-1.1.7.uf2");
        let a = r.call("GET", "/api/v1/firmware/dspi-1.1.7.uf2", &control, None);
        assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);
    }
}

#[test]
fn wp_p3rs_a_viewer_reads_but_its_writes_are_403() {
    for (mode, name) in MODES {
        let r = rig(&format!("viewer-{name}"), mode, false);
        let control = r.key("Venue script", Role::Control, false);
        let (_, snapshot) = r.group_and_snapshot(&control, -6.0);
        r.set_gain(&control, A, -1.0);
        let viewer = r.key("Dashboard", Role::Viewer, false);
        let a = r.call(
            "PUT",
            "/api/v1/groups/g1/params/out.gain?idx=0",
            &viewer,
            Some(json!({"value": -20.0})),
        );
        assert_eq!((a.status, a.code()), (403, "denied"), "{name}: {}", a.body);
        let a = r.call("POST", "/api/v1/groups/g1/presets/1/load", &viewer, None);
        assert_eq!((a.status, a.code()), (403, "denied"), "{}", a.body);
        let a = r.call(
            "POST",
            "/api/v1/groups/g1/apply",
            &viewer,
            Some(json!({"snapshot": snapshot})),
        );
        assert_eq!((a.status, a.code()), (403, "denied"), "{}", a.body);
        let a = r.call("POST", "/api/v1/runs/a1/confirm", &viewer, None);
        assert_eq!((a.status, a.code()), (403, "denied"), "{}", a.body);
        assert_eq!(r.gain(&viewer, A), -1.0, "nothing was written");
        // Reads are every role's
        for path in [
            "/api/v1/groups",
            "/api/v1/groups/g1",
            "/api/v1/rollouts",
            "/api/v1/rollouts/ro1",
            "/api/v1/firmware",
        ] {
            let a = r.call("GET", path, &viewer, None);
            assert_eq!(a.status, 200, "{path}: {}", a.body);
        }
    }
}

#[test]
fn wp_p3rs_a_guarded_group_apply_is_its_keys_and_reverts_when_another_key_tries() {
    for (mode, name) in MODES {
        let r = rig(&format!("guard-{name}"), mode, false);
        let mine = r.key("Venue script", Role::Control, false);
        let theirs = r.key("Other script", Role::Control, false);
        let (_, snapshot) = r.group_and_snapshot(&mine, -6.0);
        r.set_gain(&mine, A, -1.0);
        r.set_gain(&mine, B, -2.0);

        // Another key's confirm finds no such run, and the run reverts at its deadline.
        let a = r.call(
            "POST",
            "/api/v1/groups/g1/apply",
            &mine,
            Some(json!({"snapshot": snapshot, "guard": true, "timeout_ms": 3000})),
        );
        assert_eq!(a.status, 200, "{name}: {}", a.body);
        assert_eq!(a.rows(), vec![(A.into(), true), (B.into(), true)]);
        assert!(a.body["confirm_by"].is_string(), "{}", a.body);
        let run = a.body["run"].as_str().unwrap().to_owned();
        assert_eq!(r.gain(&theirs, A), -6.0);
        assert_eq!(r.gain(&theirs, B), -6.0);
        for end in ["confirm", "cancel"] {
            let a = r.call("POST", &format!("/api/v1/runs/{run}/{end}"), &theirs, None);
            assert_eq!((a.status, a.code()), (404, "not_found"), "{}", a.body);
        }
        wait_until("both reverted", || {
            r.gain(&theirs, A) == -1.0 && r.gain(&theirs, B) == -2.0
        });

        // The same key's confirm keeps both past their deadlines.
        let a = r.call(
            "POST",
            "/api/v1/groups/g1/apply",
            &mine,
            Some(json!({"snapshot": snapshot, "guard": true, "timeout_ms": 3000})),
        );
        assert_eq!(a.status, 200, "{}", a.body);
        let run = a.body["run"].as_str().unwrap().to_owned();
        let a = r.call("POST", &format!("/api/v1/runs/{run}/confirm"), &mine, None);
        assert_eq!(a.status, 200, "{}", a.body);
        assert_eq!(a.rows(), vec![(A.into(), true), (B.into(), true)]);
        assert_eq!(a.body["run"], run.as_str());
        std::thread::sleep(Duration::from_millis(3500));
        assert_eq!(r.gain(&theirs, A), -6.0);
        assert_eq!(r.gain(&theirs, B), -6.0);

        // And the same key's cancel reverts at once.
        r.set_gain(&mine, A, -1.0);
        r.set_gain(&mine, B, -2.0);
        let a = r.call(
            "POST",
            "/api/v1/groups/g1/apply",
            &mine,
            Some(json!({"snapshot": snapshot, "guard": true, "timeout_ms": 600000})),
        );
        assert_eq!(a.status, 200, "{}", a.body);
        let run = a.body["run"].as_str().unwrap().to_owned();
        let a = r.call("POST", &format!("/api/v1/runs/{run}/cancel"), &mine, None);
        assert_eq!(a.status, 200, "{}", a.body);
        wait_until("both cancelled", || {
            r.gain(&theirs, A) == -1.0 && r.gain(&theirs, B) == -2.0
        });
    }
}

#[test]
fn wp_p3rs_no_rollout_write_over_rest_is_405_with_allow_get() {
    for (mode, name) in MODES {
        let r = rig(&format!("405-{name}"), mode, false);
        let admin = r.key("Admin script", Role::Admin, false);
        let body = json!({"serial": A, "image": "dspi-1.1.7.uf2", "while_live": true});
        for (method, path) in [
            ("POST", "/api/v1/rollouts"),
            ("PUT", "/api/v1/rollouts"),
            ("DELETE", "/api/v1/rollouts"),
            ("POST", "/api/v1/rollouts/ro1"),
            ("POST", "/api/v1/rollouts/ro1/cancel"),
            ("POST", "/api/v1/firmware"),
            ("PUT", "/api/v1/firmware"),
            ("PUT", "/api/v1/firmware/dspi-1.1.7.uf2"),
            ("DELETE", "/api/v1/firmware/dspi-1.1.7.uf2"),
        ] {
            let a = r.call(method, path, &admin, Some(body.clone()));
            assert_eq!(a.status, 405, "{name}: {method} {path}: {}", a.body);
            assert_eq!(a.header("Allow").as_deref(), Some("GET"), "{method} {path}");
        }
        // Nothing was started, nothing was deleted.
        let a = r.call("GET", "/api/v1/rollouts", &admin, None);
        assert_eq!(
            a.body["rollouts"].as_array().unwrap().len(),
            1,
            "{}",
            a.body
        );
        let a = r.call("GET", "/api/v1/firmware", &admin, None);
        assert_eq!(a.body["images"].as_array().unwrap().len(), 1, "{}", a.body);
    }
}

#[test]
fn wp_p3rs_the_rollouts_reads_are_tls_only_for_every_key_and_mode() {
    for (compat, name) in [(false, "on"), (true, "compat")] {
        let r = rig(&format!("tls-{name}"), TlsMode::On, compat);
        let legacy = r.key("Legacy script", Role::Admin, true);
        // The groups are served in the clear to a plaintext_ok key ...
        let a = r.call_plain("GET", "/api/v1/groups", &legacy);
        assert_eq!(a.status, 200, "{name}: {}", a.body);
        // ... the rollout's reads never, before or after a TLS request of the same key.
        for round in 0..2 {
            for path in [
                "/api/v1/rollouts",
                "/api/v1/rollouts/ro1",
                "/api/v1/firmware",
            ] {
                let a = r.call_plain("GET", path, &legacy);
                assert_eq!(
                    (a.status, a.code()),
                    (426, "tls_required"),
                    "{name} {round}: {path}: {}",
                    a.body
                );
                assert_eq!(a.header("Upgrade").as_deref(), Some("TLS/1.3"));
            }
            let a = r.call("GET", "/api/v1/firmware", &legacy, None);
            assert_eq!(a.status, 200, "{name} {round}: {}", a.body);
        }
        // In upstream compatibility a key not minted plaintext_ok works in the clear too,
        // but not for the rollout.
        if compat {
            let strict = r.key("Strict script", Role::Viewer, false);
            assert_eq!(r.call_plain("GET", "/api/v1/groups", &strict).status, 200);
            let a = r.call_plain("GET", "/api/v1/firmware", &strict);
            assert_eq!((a.status, a.code()), (426, "tls_required"), "{}", a.body);
        }
    }
}
