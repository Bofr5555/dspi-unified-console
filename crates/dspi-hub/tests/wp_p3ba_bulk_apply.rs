//! WP P3BA's done criteria through the real server with two demo devices: `group.apply`
//! (one shared snapshot on every member of a group, each as the caller's child session),
//! with and without a guard per device, `group.apply.confirm` and `group.apply.cancel`
//! by `run`, a Control caller's `image_refused` row on a limited device, the session's
//! close reverting what is pending, and a `recall` record per device naming the caller.
//! (The 256-row bound is `api::bulk`'s unit test `wp_p3ba_a_run_past_256_rows_is_bad_request`:
//! a group holds 64 members at most, so only a fake resolver gets past it.)
// -- WP P3RS -- the family exists only with feature `api`.
#![cfg(feature = "api")]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::{AccessMode, BusMessage, Hub, HubConfig, NetworkConfig, Role, ServiceConfig};
use dspi_session::events::EventKind;
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(30);

// ── The log, kept to look for image bytes ──────────────────────────────────

struct Keep;

fn kept() -> &'static Mutex<Vec<String>> {
    static K: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    K.get_or_init(|| Mutex::new(Vec::new()))
}

impl log::Log for Keep {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        // Ours only: the WebSocket library's trace dumps every frame, the test's own too.
        if !record.target().starts_with("dspi") {
            return;
        }
        kept().lock().unwrap().push(format!(
            "{} {}: {}",
            record.level(),
            record.target(),
            record.args()
        ));
    }

    fn flush(&self) {}
}

fn keep_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        static KEEP: Keep = Keep;
        let _ = log::set_logger(&KEEP);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// The longest run of base64 characters in any kept log line, and the line's start.
fn longest_base64_run() -> (usize, String) {
    let lines = kept().lock().unwrap();
    let mut longest = (0, String::new());
    for l in lines.iter() {
        let mut run = 0;
        for c in l.chars() {
            if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' {
                run += 1;
                if run > longest.0 {
                    longest = (run, l.chars().take(160).collect());
                }
            } else {
                run = 0;
            }
        }
    }
    longest
}

// ── The rig ─────────────────────────────────────────────────────────────────

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3ba").join(format!(
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

struct Rig {
    hub: Hub,
    dir: PathBuf,
}

const A: &str = "P3BA-A";
const B: &str = "P3BA-B";

/// A hub with groups, the inventory and the snapshot store, `A` and `B` plugged.
fn rig(tag: &str) -> Rig {
    keep_logs();
    let dir = scratch(tag);
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            inventory: true,
            groups: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p3ba")
    })
    .unwrap();
    let r = Rig { hub, dir };
    r.plug(A);
    r.plug(B);
    r
}

impl Rig {
    fn plug(&self, serial: &str) -> u8 {
        self.hub
            .attach(Box::new(FakeDevice::demo().with_serial(serial)), 0)
            .unwrap()
    }

    fn handle(&self, serial: &str) -> u8 {
        self.hub
            .devices()
            .iter()
            .find(|d| d.identity.serial == serial)
            .unwrap()
            .handle
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
            "client": {"name": name, "app": "wp_p3ba", "version": "0"}}));
        let hello = ws.next("hello", |_| true);
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

    /// `A`'s state now as a snapshot named `name` in the hub's store: its id. `A`'s
    /// output 1 gain is set to `gain` first.
    fn snapshot_of_a(&self, ws: &mut Ws, gain: f64, name: &str) -> String {
        assert!(ok(&ws.set(A, gain)));
        let core = DspiCore::new();
        let t = self.hub.handle().open_local(A).unwrap();
        core.open_transport(Box::new(t), A.into(), 0).unwrap();
        wait_until("the mirror", || {
            core.process_notifications();
            core.get_state().output_gain_db[0] == gain as f32
        });
        let _ = core.feature_present("device_name");
        let text = core
            .snapshot_capture_document(name.into(), String::new())
            .unwrap();
        core.disconnect();
        let store = SnapshotStore::new(self.dir.join(format!("client-{name}")));
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        // A time of its own, so two snapshots never share an id in the hub's store.
        static SAVED: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        let at = 1_763_200_000_000 + SAVED.fetch_add(60_000, Ordering::SeqCst);
        let e = store.save(A, &mut doc, at).unwrap();
        let (_, filed) = store.read(&e.id).unwrap();
        let compact: Value = serde_json::from_str(&filed).unwrap();
        let compact = compact.to_string();
        if compact.len() < 60_000 {
            let put = ws.ask(json!({"t": "snapshot.put", "serial": A, "document": compact}));
            assert_eq!(put["t"], "ok", "{put}");
        } else {
            dspi_hub::snapshots::HubSnapshots::open(&self.dir.join("store"))
                .put(A, &filed)
                .unwrap();
        }
        e.id
    }

    /// Group `g1` of `A` and `B`, set by an Admin.
    fn group(&self) -> String {
        let mut admin = self.client("Installer", Role::Admin);
        let g = admin.ask(json!({"t": "group.set", "group": {"name": "Hall", "members": [A, B]}}));
        assert!(ok(&g), "{g}");
        g["group"]["id"].as_str().unwrap().to_owned()
    }
}

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    caps: Vec<String>,
    seen: Vec<Value>,
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
            seen: Vec::new(),
        }
    }

    fn send(&mut self, v: Value) {
        self.ws.send(Message::text(v.to_string())).unwrap();
    }

    fn read(&mut self) -> Value {
        loop {
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn next(&mut self, t: &str, pick: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.seen.iter().position(|v| v["t"] == t && pick(v)) {
            return self.seen.remove(i);
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            let v = self.read();
            if v["t"] == t && pick(&v) {
                return v;
            }
            self.seen.push(v);
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
            self.seen.push(v);
        }
    }

    /// Output 1's gain on `serial`.
    fn set(&mut self, serial: &str, gain: f64) -> Value {
        self.ask(
            json!({"t": "param.set", "serial": serial, "path": "out.gain", "idx": [0],
            "value": gain}),
        )
    }

    fn gain(&mut self, serial: &str) -> f64 {
        let v = self.ask(
            json!({"t": "param.get", "serial": serial, "path": "out.gain",
            "idx": [0]}),
        );
        assert!(ok(&v), "{v}");
        v["value"].as_f64().unwrap()
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

/// The rows' (serial, ok, code).
fn rows(v: &Value) -> Vec<(String, bool, String)> {
    v["results"]
        .as_array()
        .unwrap_or_else(|| panic!("no results: {v}"))
        .iter()
        .map(|r| {
            (
                r["serial"].as_str().unwrap().to_owned(),
                r["ok"].as_bool().unwrap(),
                r["code"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn all_ok(v: &Value, serials: &[&str]) {
    let want: Vec<(String, bool, String)> = serials
        .iter()
        .map(|s| ((*s).to_owned(), true, String::new()))
        .collect();
    assert_eq!(rows(v), want, "{v}");
}

// ── The done criteria ───────────────────────────────────────────────────────

#[test]
fn wp_p3ba_an_apply_without_a_guard_restores_both_and_records_a_recall_each() {
    let r = rig("plain");
    let g = r.group();
    let mut c = r.client("Venue tablet", Role::Control);
    assert!(c.caps.iter().any(|c| c == "group_apply"), "{:?}", c.caps);
    let id = r.snapshot_of_a(&mut c, -6.0, "Doors open");
    assert!(ok(&c.set(A, -1.0)));
    assert!(ok(&c.set(B, -2.0)));

    let sub = r.hub.bus().subscribe("p3ba-history", 4096);
    let v = c.ask(json!({"t": "group.apply", "group": g, "name": "Doors open"}));
    assert!(ok(&v), "{v}");
    assert_eq!(v["v"], 1);
    all_ok(&v, &[A, B]);
    assert!(v["run"].as_str().unwrap().starts_with('a'), "{v}");
    assert!(v.get("confirm_by").is_none(), "no guard, no deadline: {v}");
    for row in v["results"].as_array().unwrap() {
        assert_eq!(row["image"], true, "{row}");
        assert!(row["lines"].is_array() && row["notes"].is_array(), "{row}");
        assert!(row.get("guard").is_none(), "{row}");
    }
    assert_eq!(c.gain(A), -6.0);
    assert_eq!(c.gain(B), -6.0, "A's snapshot applied to B too");

    // A recall record per device, with the caller's name.
    let mut recorded = Vec::new();
    let start = Instant::now();
    while recorded.len() < 2 {
        assert!(start.elapsed() < WAIT, "recall records: {recorded:?}");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Event { serial, record } = &*m
            && record.kind == EventKind::Recall
        {
            assert_eq!(record.by.as_deref(), Some("Venue tablet"), "{record:?}");
            assert!(!record.lines.is_empty(), "{record:?}");
            recorded.push(serial.clone());
        }
    }
    recorded.sort();
    assert_eq!(recorded, vec![A.to_owned(), B.to_owned()]);

    // By id, and by targets in their order; the same over again is fine.
    let v = c.ask(json!({"t": "group.apply", "targets": [B, A], "snapshot": id}));
    all_ok(&v, &[B, A]);
    // The run without a guard settles nothing: no rows.
    let run = v["run"].as_str().unwrap();
    let conf = c.ask(json!({"t": "group.apply.confirm", "run": run}));
    assert_eq!(conf["results"], json!([]), "{conf}");
    r.hub.stop();
}

#[test]
fn wp_p3ba_guarded_both_revert_at_their_deadlines_and_one_confirm_keeps_both() {
    let r = rig("guard");
    let g = r.group();
    let mut c = r.client("Venue tablet", Role::Control);
    let mut w = r.client("Watcher", Role::Viewer);
    r.snapshot_of_a(&mut c, -6.0, "Quiet");
    assert!(ok(&c.set(A, -1.0)));
    assert!(ok(&c.set(B, -2.0)));

    // Nobody confirms: each device goes back to its own state at its deadline.
    let v = c.ask(
        json!({"t": "group.apply", "group": g, "name": "Quiet", "guard": true,
        "timeout_ms": 4000}),
    );
    all_ok(&v, &[A, B]);
    assert!(v["confirm_by"].as_str().unwrap().ends_with('Z'), "{v}");
    for row in v["results"].as_array().unwrap() {
        assert!(row["guard"].as_u64().is_some(), "{row}");
    }
    assert_eq!(w.gain(A), -6.0);
    assert_eq!(w.gain(B), -6.0);
    for s in [A, B] {
        let ended = w.next("apply.ended", |e| e["serial"] == s);
        assert_eq!(ended["result"], "reverted", "{ended}");
        assert_eq!(ended["reason"], "timeout", "{ended}");
    }
    wait_until("both reverted", || w.gain(A) == -1.0 && w.gain(B) == -2.0);
    // Confirming the run now: each guard had already reverted.
    let run = v["run"].as_str().unwrap().to_owned();
    let late = c.ask(json!({"t": "group.apply.confirm", "run": run}));
    assert!(ok(&late), "{late}");
    assert_eq!(
        rows(&late),
        vec![
            (A.into(), false, "conflict".into()),
            (B.into(), false, "conflict".into())
        ]
    );

    // One confirm keeps both, past their deadlines.
    let v = c.ask(
        json!({"t": "group.apply", "group": g, "name": "Quiet", "guard": true,
        "timeout_ms": 2000}),
    );
    all_ok(&v, &[A, B]);
    let run = v["run"].as_str().unwrap().to_owned();
    let conf = c.ask(json!({"t": "group.apply.confirm", "run": run}));
    all_ok(&conf, &[A, B]);
    assert_eq!(conf["run"], run.as_str());
    for s in [A, B] {
        let ended = w.next("apply.ended", |e| e["serial"] == s);
        assert_eq!(ended["result"], "kept", "{ended}");
    }
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(w.gain(A), -6.0);
    assert_eq!(w.gain(B), -6.0);
    // Settled: the same rows again, for a confirm or a cancel.
    let again = c.ask(json!({"t": "group.apply.cancel", "run": run}));
    assert_eq!(again["results"], conf["results"], "{again}");
    // Another session's run, or none, is not found.
    let mut other = r.client("Second tablet", Role::Control);
    let theirs = other.ask(json!({"t": "group.apply.confirm", "run": run}));
    assert_eq!(code(&theirs), "not_found", "{theirs}");
    let none = c.ask(json!({"t": "group.apply.cancel", "run": "a999"}));
    assert_eq!(code(&none), "not_found", "{none}");
    r.hub.stop();
}

#[test]
fn wp_p3ba_a_cancel_reverts_both_now() {
    let r = rig("cancel");
    let g = r.group();
    let mut c = r.client("Venue tablet", Role::Control);
    r.snapshot_of_a(&mut c, -6.0, "Quiet");
    assert!(ok(&c.set(A, -1.0)));
    assert!(ok(&c.set(B, -2.0)));
    let v = c.ask(
        json!({"t": "group.apply", "group": g, "name": "Quiet", "guard": true,
        "timeout_ms": 600000}),
    );
    all_ok(&v, &[A, B]);
    assert_eq!(c.gain(B), -6.0);
    // While pending, a second apply on the group is a conflict on each row.
    let again = c.ask(json!({"t": "group.apply", "group": g, "name": "Quiet"}));
    assert_eq!(
        rows(&again),
        vec![
            (A.into(), false, "conflict".into()),
            (B.into(), false, "conflict".into())
        ]
    );
    let run = v["run"].as_str().unwrap();
    let cancel = c.ask(json!({"t": "group.apply.cancel", "run": run}));
    all_ok(&cancel, &[A, B]);
    for row in cancel["results"].as_array().unwrap() {
        assert_eq!(row["reverted"], true, "{row}");
    }
    // Written back at once, on the cancel's own thread.
    assert_eq!(c.gain(A), -1.0);
    assert_eq!(c.gain(B), -2.0);
    r.hub.stop();
}

#[test]
fn wp_p3ba_a_control_caller_on_a_limited_device_gets_image_refused_there_only() {
    let r = rig("limits");
    let g = r.group();
    let mut admin = r.client("Installer", Role::Admin);
    let mut c = r.client("Venue tablet", Role::Control);
    r.snapshot_of_a(&mut c, -6.0, "Doors open");
    assert!(ok(&c.set(A, -1.0)));
    assert!(ok(&c.set(B, -2.0)));
    let lim = admin.ask(json!({"t": "limits.set", "handle": r.handle(B),
        "profile": {"outputs": [{"output": 0, "maxGainDb": 3.0}]}}));
    assert!(ok(&lim), "{lim}");

    let v = c.ask(json!({"t": "group.apply", "group": g, "name": "Doors open"}));
    assert_eq!(
        rows(&v),
        vec![
            (A.into(), true, String::new()),
            (B.into(), false, "image_refused".into())
        ],
        "{v}"
    );
    assert_eq!(c.gain(A), -6.0);
    assert_eq!(c.gain(B), -2.0, "nothing written to the limited device");
    r.hub.stop();
}

#[test]
fn wp_p3ba_the_session_closing_reverts_what_is_pending() {
    let r = rig("close");
    let g = r.group();
    let mut w = r.client("Watcher", Role::Viewer);
    {
        let mut c = r.client("Venue tablet", Role::Control);
        r.snapshot_of_a(&mut c, -6.0, "Quiet");
        assert!(ok(&c.set(A, -1.0)));
        assert!(ok(&c.set(B, -2.0)));
        let v = c.ask(
            json!({"t": "group.apply", "group": g, "name": "Quiet", "guard": true,
            "timeout_ms": 600000}),
        );
        all_ok(&v, &[A, B]);
        assert_eq!(w.gain(A), -6.0);
        assert_eq!(w.gain(B), -6.0);
        // The client goes.
        c.ws.close(None).unwrap();
        let _ = c.ws.flush();
    }
    for s in [A, B] {
        let ended = w.next("apply.ended", |e| e["serial"] == s);
        assert_eq!(ended["reason"], "session_closed", "{ended}");
        assert_eq!(ended["result"], "reverted", "{ended}");
    }
    wait_until("both reverted", || w.gain(A) == -1.0 && w.gain(B) == -2.0);
    r.hub.stop();
}

#[test]
fn wp_p3ba_refusals_roles_sources_targets_offline_and_no_image_bytes_in_the_log() {
    let r = rig("refusals");
    let g = r.group();
    let mut admin = r.client("Installer", Role::Admin);
    let mut c = r.client("Venue tablet", Role::Control);
    let mut viewer = r.client("Watcher", Role::Viewer);
    let id = r.snapshot_of_a(&mut c, -6.0, "Doors open");

    let v = viewer.ask(json!({"t": "group.apply", "group": g, "snapshot": id}));
    assert_eq!(code(&v), "denied", "{v}");
    let v = viewer.ask(json!({"t": "group.apply.confirm", "run": "a1"}));
    assert_eq!(code(&v), "denied", "{v}");
    let bad = |c: &mut Ws, m: Value| {
        let v = c.ask(m);
        assert_eq!(code(&v), "bad_request", "{v}");
    };
    bad(&mut c, json!({"t": "group.apply", "group": g}));
    bad(
        &mut c,
        json!({"t": "group.apply", "group": g, "snapshot": id, "name": "Doors open"}),
    );
    bad(
        &mut c,
        json!({"t": "group.apply", "group": g, "targets": [A], "snapshot": id}),
    );
    bad(
        &mut c,
        json!({"t": "group.apply", "targets": [A, A], "snapshot": id}),
    );
    let many: Vec<String> = (0..17).map(|i| format!("P3BA-{i}")).collect();
    bad(
        &mut c,
        json!({"t": "group.apply", "targets": many, "snapshot": id}),
    );
    bad(
        &mut c,
        json!({"t": "group.apply", "group": g, "snapshot": id, "timeout_ms": 10}),
    );
    bad(
        &mut c,
        json!({"t": "group.apply", "group": g, "document": {"meta": {"kind": "preset"}}}),
    );
    let v = c.ask(json!({"t": "group.apply", "group": "g99", "snapshot": id}));
    assert_eq!(code(&v), "not_found", "{v}");
    let v = c.ask(json!({"t": "group.apply", "group": g, "name": "Nope"}));
    assert_eq!(code(&v), "not_found", "{v}");

    // Two snapshots of one name in the store: conflict.
    let _ = r.snapshot_of_a(&mut c, -4.0, "Twice");
    let _ = r.snapshot_of_a(&mut c, -5.0, "Twice");
    let v = c.ask(json!({"t": "group.apply", "group": g, "name": "Twice"}));
    assert_eq!(code(&v), "conflict", "{v}");

    // A document as the JSON object; an unknown target and an empty group are rows.
    let (_, text) = dspi_hub::snapshots::HubSnapshots::open(&r.dir.join("store"))
        .get(&id)
        .unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert!(ok(&c.set(B, -2.0)));
    let v = c.ask(json!({"t": "group.apply", "targets": [B, "P3BA-NOPE"], "document": doc}));
    assert_eq!(
        rows(&v),
        vec![
            (B.into(), true, String::new()),
            ("P3BA-NOPE".into(), false, "not_found".into())
        ],
        "{v}"
    );
    assert_eq!(c.gain(B), -6.0);
    let empty = admin.ask(json!({"t": "group.set", "group": {"name": "Nobody", "members": []}}));
    let empty = empty["group"]["id"].as_str().unwrap().to_owned();
    let v = c.ask(json!({"t": "group.apply", "group": empty, "snapshot": id}));
    assert_eq!(v["results"], json!([]), "{v}");

    // An offline member fails at once and the other applies.
    r.hub.detach(r.handle(B)).unwrap();
    assert!(ok(&c.set(A, -1.0)));
    let v = c.ask(
        json!({"t": "group.apply", "group": g, "snapshot": id, "guard": true,
        "timeout_ms": 600000}),
    );
    assert_eq!(
        rows(&v),
        vec![
            (A.into(), true, String::new()),
            (B.into(), false, "offline".into())
        ],
        "{v}"
    );
    let conf = c.ask(json!({"t": "group.apply.confirm", "run": v["run"]}));
    all_ok(&conf, &[A]);

    // No image bytes (a base64 run) in anything the hub logged.
    assert!(!kept().lock().unwrap().is_empty(), "the hub's log was kept");
    let (run, line) = longest_base64_run();
    assert!(
        run < 128,
        "a base64-like run of {run} characters in the log: {line}"
    );
    r.hub.stop();
}
