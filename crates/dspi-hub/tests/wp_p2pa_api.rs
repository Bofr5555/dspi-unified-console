//! WP P2PA's done criteria, through the real server on the demo device: the parameter
//! API (`param.*`), presets by name (`preset.*`), hub-side snapshot recall
//! (`snapshot.recall`) and the `snapshot` family's registration, each over a WebSocket of
//! a paired client with its role.
//!
//! The two session-id tests (PLAN-P2 M2): a Control child session's write past a limits
//! profile is `limit_exceeded` and nothing reaches the device; a raw `EXT_CORE_VREG`
//! write from a child session is `DENIED` by the device's worker.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, ParamValue};
use dspi_hub::session::{
    FIRST_REMOTE_SESSION_ID, GUARD_SESSION_ID, INTERNAL_SESSION_ID, LOCAL_SESSION_ID,
};
use dspi_hub::{
    AccessMode, BusMessage, Hub, HubClock, HubConfig, HubEvent, HubListener, NetworkConfig, Role,
    ServiceConfig, TestClock,
};
use dspi_proto::ext::VregOp;
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_session::events::EventKind;
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::{FakeDevice, Transport, TransportError};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/wp-P2PA/tmp, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2pa").join(format!(
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

/// The hub's events, for the sessions that open and close.
#[derive(Default)]
struct Events(Mutex<Vec<HubEvent>>);

impl HubListener for Events {
    fn on_event(&self, event: HubEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn closed(&self, id: u16) -> bool {
        self.0
            .lock()
            .unwrap()
            .contains(&HubEvent::SessionClosed { id })
    }
}

struct Rig {
    hub: Hub,
    dir: PathBuf,
    events: Arc<Events>,
}

fn rig_with(tag: &str, clock: Option<Arc<dyn HubClock>>) -> Rig {
    let dir = scratch(tag);
    let mut services = ServiceConfig::default();
    if let Some(clock) = clock {
        services.clock = clock;
    }
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services,
        ..HubConfig::in_process("p2pa")
    })
    .unwrap();
    let events = Arc::new(Events::default());
    hub.set_listener(Some(Arc::clone(&events) as Arc<dyn HubListener>));
    Rig { hub, dir, events }
}

fn rig(tag: &str) -> Rig {
    rig_with(tag, None)
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
            "client": {"name": name, "app": "wp_p2pa", "version": "0"}}));
        let hello = ws.next("hello");
        ws.caps = hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws.session = ok["session"].as_u64().unwrap() as u16;
        ws
    }

    /// The app's own core on the device, through the hub (another client's mirror).
    fn app_core(&self, serial: &str) -> Arc<DspiCore> {
        let core = DspiCore::new();
        let t = self.hub.handle().open_local(serial).unwrap();
        core.open_transport(Box::new(t), serial.into(), 0).unwrap();
        core
    }

    /// A snapshot of the device as it is now, filed as a client's store files it, then
    /// put into the hub's store: its id. Over the socket when the document fits one
    /// DSPi Link frame, else straight into the store the hub's service reads.
    fn put_snapshot(&self, ws: &mut Ws, core: &DspiCore, serial: &str, name: &str) -> String {
        // The core over the hub finishes its probe after the image (LK4): the names
        // outside the image are read once it has.
        let _ = core.feature_present("device_name");
        let text = core
            .snapshot_capture_document(name.into(), String::new())
            .unwrap();
        let store = SnapshotStore::new(self.dir.join(format!("client-{name}")));
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        let e = store.save(serial, &mut doc, 1_763_200_000_000).unwrap();
        let (_, filed) = store.read(&e.id).unwrap();
        let compact: Value = serde_json::from_str(&filed).unwrap();
        let compact = compact.to_string();
        if compact.len() < 60_000 {
            let put = ws.ask(json!({"t": "snapshot.put", "serial": serial, "document": compact}));
            assert_eq!(put["t"], "ok", "{put}");
        } else {
            dspi_hub::snapshots::HubSnapshots::open(&self.dir.join("store"))
                .put(serial, &filed)
                .unwrap();
            // The family over the socket, with its event.
            let note = ws.ask(json!({"t": "snapshot.note", "snapshot": e.id, "note": "put"}));
            assert_eq!(note["t"], "ok", "{note}");
        }
        e.id
    }
}

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    session: u16,
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
            session: 0,
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

    fn next(&mut self, t: &str) -> Value {
        if let Some(i) = self.seen.iter().position(|v| v["t"] == t) {
            return self.seen.remove(i);
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            let v = self.read();
            if v["t"] == t {
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
            if v["id"] == id {
                return v;
            }
            self.seen.push(v);
        }
    }

    fn set(&mut self, serial: &str, path: &str, idx: Value, value: Value) -> Value {
        self.ask(
            json!({"t": "param.set", "serial": serial, "path": path, "idx": idx, "value": value}),
        )
    }

    fn get(&mut self, serial: &str, path: &str, idx: Value) -> Value {
        let v = self.ask(json!({"t": "param.get", "serial": serial, "path": path, "idx": idx}));
        assert_eq!(v["t"], "ok", "{v}");
        v
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

/// Output 1's gain at most 3 dB, the master volume at most -10 dB.
fn gain_profile() -> Value {
    json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "masterVolumeMaxDb": -10.0})
}

/// The session of the next notification from `serial`, and its `by`.
fn next_write(sub: &dspi_hub::Subscription, serial: &str) -> (u16, Option<String>) {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no notification from {serial}");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Notification {
                serial: s,
                session,
                by,
                ..
            } = &*m
            && s == serial
            && *session != 0
        {
            return (*session, by.clone());
        }
    }
}

#[test]
fn wp_p2pa_list_get_and_set_a_float_a_bool_an_eq_field_and_a_choice() {
    let r = rig("params");
    let serial = "P2PA-PARAMS";
    r.plug(serial);
    let mut c = r.client("Front of house", Role::Control);
    assert!(c.caps.iter().any(|x| x == "param_api"), "{:?}", c.caps);
    assert!(c.caps.iter().any(|x| x == "hub_snapshots"), "{:?}", c.caps);

    let list = c.ask(json!({"t": "param.list", "serial": serial}));
    assert!(ok(&list), "{list}");
    assert_eq!(list["v"], 1);
    let rows = list["params"].as_array().unwrap();
    let row = |p: &str| rows.iter().find(|r| r["path"] == p).cloned().unwrap();
    let vol = row("vol.master");
    assert_eq!(vol["kind"], "float");
    assert_eq!(vol["routed"], true);
    let eq = row("eq.freq");
    assert_eq!(eq["indices"], 2);
    assert_eq!(eq["layout"][0]["name"], "channel");
    assert!(eq["layout"][0]["count"].as_u64().unwrap() > 0, "{eq}");
    assert_eq!(row("out.gain")["layout"][0]["name"], "output");
    assert_eq!(row("dev.reset")["routed"], false);
    assert_eq!(
        row("meters.clear")["routed"],
        false,
        "the hub never clears the clip latch"
    );
    assert_eq!(row("lev.speed")["kind"], "choice");

    // A float.
    let set = c.set(serial, "vol.master", json!([]), json!(-20.0));
    assert!(ok(&set), "{set}");
    assert_eq!(set["value"], -20.0, "the value read back");
    assert_eq!(c.get(serial, "vol.master", json!([]))["value"], -20.0);
    // A bool.
    let set = c.set(serial, "vol.mute", json!(null), json!(true));
    assert_eq!(set["value"], true, "{set}");
    assert_eq!(c.get(serial, "vol.mute", json!(null))["value"], true);
    // An indexed EQ field: one field of one band, the rest stays.
    let q = c.get(serial, "eq.q", json!([0, 4]))["value"].clone();
    let set = c.set(serial, "eq.freq", json!([0, 4]), json!(1234.0));
    assert_eq!(set["value"], 1234.0, "{set}");
    assert_eq!(c.get(serial, "eq.freq", json!([0, 4]))["value"], 1234.0);
    assert_eq!(c.get(serial, "eq.q", json!([0, 4]))["value"], q);
    // A choice by name, answered with both.
    let set = c.set(serial, "lev.speed", json!([]), json!("fast"));
    assert_eq!(
        (set["value"].clone(), set["choice"].clone()),
        (json!(2), json!("fast")),
        "{set}"
    );
    let got = c.get(serial, "lev.speed", json!([]));
    assert_eq!(got["choice"], "fast", "{got}");

    // Many at once, a bad one among them.
    let many = c.ask(json!({"t": "param.get_many", "serial": serial, "items": [
        {"path": "vol.master"}, {"path": "no.such"}, {"path": "eq.freq", "idx": [0, 4]}]}));
    let values = many["values"].as_array().unwrap();
    assert_eq!(values[0]["value"], -20.0, "{many}");
    assert_eq!(
        (values[1]["ok"].clone(), code(&values[1])),
        (json!(false), "bad_request")
    );
    assert_eq!(values[2]["value"], 1234.0);

    // Outside the registry's range: bad_request, with the range, and nothing sent.
    let out = c.set(serial, "vol.master", json!([]), json!(40.0));
    assert_eq!(code(&out), "bad_request", "{out}");
    assert!(
        out.get("min").is_some() && out.get("max").is_some(),
        "{out}"
    );
    assert_eq!(c.get(serial, "vol.master", json!([]))["value"], -20.0);
    // Paths the API does not write, and no raw opcode.
    for (path, idx, value) in [
        ("meters.clear", json!([]), json!(null)),
        ("bulk", json!([]), json!(null)),
        ("dev.reset", json!([]), json!(null)),
        ("ext.vreg", json!([]), json!(null)),
    ] {
        let v = c.set(serial, path, idx, value);
        assert_eq!(code(&v), "bad_request", "{path}: {v}");
    }
    // The wrong number of indices; an index the device has not.
    assert_eq!(
        code(&c.set(serial, "out.gain", json!([]), json!(0.0))),
        "bad_request"
    );
    assert_eq!(
        code(&c.set(serial, "out.gain", json!([99]), json!(0.0))),
        "bad_request"
    );

    // A Viewer reads, never writes.
    let mut v = r.client("Watcher", Role::Viewer);
    assert_eq!(v.get(serial, "vol.master", json!([]))["value"], -20.0);
    assert!(ok(&v.ask(json!({"t": "param.list", "serial": serial}))));
    let denied = v.set(serial, "vol.master", json!([]), json!(-30.0));
    assert_eq!(code(&denied), "denied", "{denied}");
    assert_eq!(c.get(serial, "vol.master", json!([]))["value"], -20.0);
    // Addressing.
    let unknown = c.ask(json!({"t": "param.get", "serial": "NOPE", "path": "vol.master"}));
    assert_eq!(code(&unknown), "not_found");
    let none = c.ask(json!({"t": "param.get", "path": "vol.master"}));
    assert_eq!(code(&none), "bad_request");
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_control_childs_set_past_a_limit_is_limit_exceeded_and_nothing_is_sent() {
    let r = rig("limit");
    let serial = "P2PA-LIMIT";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut c = r.client("Venue tablet", Role::Control);
    let set = admin.ask(json!({"t": "limits.set", "handle": handle, "profile": gain_profile()}));
    assert!(ok(&set), "{set}");
    let before = c.get(serial, "out.gain", json!([0]))["value"].clone();
    let sub = r.hub.bus().subscribe("p2pa-test", 1024);

    let past = c.set(serial, "out.gain", json!([0]), json!(6.0));
    assert_eq!(code(&past), "limit_exceeded", "{past}");
    assert_eq!(past["limit"], "out_gain", "{past}");
    assert_eq!(
        c.get(serial, "out.gain", json!([0]))["value"],
        before,
        "nothing reached the device"
    );
    assert!(
        sub.recv_timeout(Duration::from_millis(300))
            .is_none_or(|m| !matches!(
                &*m,
                BusMessage::Notification { serial: s, .. } if s == serial
            )),
        "no write went out"
    );
    let vol = c.set(serial, "vol.master", json!([]), json!(-3.0));
    assert_eq!(
        (code(&vol), vol["limit"].as_str()),
        ("limit_exceeded", Some("master_volume"))
    );
    // Within the limit: written.
    let within = c.set(serial, "out.gain", json!([0]), json!(2.0));
    assert_eq!(within["value"], 2.0, "{within}");
    // An Admin's child is held to the profile too.
    let past = admin.set(serial, "out.gain", json!([0]), json!(6.0));
    assert_eq!(code(&past), "limit_exceeded", "{past}");
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_raw_core_vreg_write_from_a_child_session_is_denied() {
    let r = rig("vreg");
    let device = FakeDevice::demo().with_serial("P2PA-VREG");
    device.remote().vreg_set_build_default(0x0D); // 1.20 V
    r.hub.attach(Box::new(device), 0x2E8B).unwrap();
    let h = r.hub.handle();
    let raise = VregOp::Set(0x0E).encode(); // 1.25 V
    for role in [Role::Control, Role::Admin] {
        let parent = h.open_session(role, None).unwrap();
        let child = h.open_child(&parent).unwrap();
        // The session-id rule: the child's own id, from the remote range.
        assert!(child.id() >= FIRST_REMOTE_SESSION_ID);
        assert_ne!(child.id(), parent.id());
        assert!(![LOCAL_SESSION_ID, INTERNAL_SESSION_ID, GUARD_SESSION_ID].contains(&child.id()));
        assert_eq!(child.session().role(), role);
        let mut t = h.open_as("P2PA-VREG", &child, false).unwrap();
        assert_eq!(t.session_id(), Some(child.id()), "never LOCAL_SESSION_ID");
        let refused = t.control_out(op::REQ_EXTENSION, g::EXT_CORE_VREG, &raise);
        assert!(
            matches!(refused, Err(TransportError::Denied(_))),
            "{role:?}: {refused:?}"
        );
        h.close_session(child.id());
        h.close_session(parent.id());
    }
    // Nothing was stored: the unit reports no stored raise.
    let parent = h.open_session(Role::Admin, None).unwrap();
    let child = h.open_child(&parent).unwrap();
    let mut t = h.open_as("P2PA-VREG", &child, false).unwrap();
    let len = dspi_proto::ext::CoreVregStatus::SIZE as u16;
    let status = t
        .control_in(op::REQ_EXTENSION, g::EXT_CORE_VREG, len)
        .unwrap();
    let stored = dspi_proto::ext::CoreVregStatus::decode(&status)
        .unwrap()
        .stored_sel;
    assert_ne!(stored, 0x0E, "nothing was written");
    drop(t);
    h.close_session(child.id());
    // And the API itself has no way to the voltage.
    r.plug("P2PA-VREG2");
    let mut admin = r.client("Installer", Role::Admin);
    let v = admin.set("P2PA-VREG2", "ext.vreg", json!([]), json!(0x0E));
    assert_eq!(code(&v), "bad_request", "{v}");
    r.hub.stop();
}

#[test]
fn wp_p2pa_the_child_session_is_remote_and_its_write_is_attributed() {
    let r = rig("attribution");
    let serial = "P2PA-ATTR";
    r.plug(serial);
    let mut c = r.client("Front of house", Role::Control);
    let sub = r.hub.bus().subscribe("p2pa-attr", 1024);
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-25.0))));
    let (session, by) = next_write(&sub, serial);
    assert!(session >= FIRST_REMOTE_SESSION_ID, "{session}");
    assert_ne!(session, c.session, "a child session of its own");
    assert!(![LOCAL_SESSION_ID, INTERNAL_SESSION_ID, GUARD_SESSION_ID].contains(&session));
    assert_eq!(by.as_deref(), Some("Front of house"), "the client's name");
    r.hub.stop();
}

#[test]
fn wp_p2pa_an_api_write_is_foreign_in_another_mirror_and_changed_elsewhere_in_its_undo() {
    let r = rig("echo");
    let serial = "P2PA-ECHO";
    r.plug(serial);
    let app = r.app_core(serial);
    app.write_param("vol.master".into(), vec![], ParamValue::Float(-17.5))
        .unwrap();
    assert_eq!(app.history(8).len(), 1, "the app's own change, to undo");
    let mut c = r.client("Venue tablet", Role::Control);
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-30.0))));
    wait_until("the app's mirror to follow", || {
        app.process_notifications();
        app.get_state().master_volume_db == -30.0
    });
    let undo = app.undo().unwrap();
    let note = undo.note.clone().unwrap_or_default();
    assert!(
        note.contains("was changed by"),
        "changed elsewhere: {undo:?} {:?}",
        app.history(8)
    );
    assert_eq!(
        app.get_state().master_volume_db,
        -30.0,
        "left as they set it"
    );
    app.disconnect();
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_locked_devices_control_client_sets_volume_mute_and_preset_only() {
    let r = rig("locked");
    let serial = "P2PA-LOCK";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let saved = admin.ask(
        json!({"t": "preset.save", "serial": serial, "slot": 0, "name": "House", "confirm": true}),
    );
    assert!(ok(&saved), "{saved}");
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq", "routing"]);
    assert!(ok(&admin.ask(
        json!({"t": "limits.set", "handle": handle, "profile": p})
    )));
    let pin = admin.ask(json!({"t": "limits.pin", "handle": handle, "pin": "2468"}));
    assert!(ok(&pin), "{pin}");

    let mut c = r.client("Venue tablet", Role::Control);
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-20.0))));
    assert!(ok(&c.set(serial, "vol.mute", json!([]), json!(true))));
    assert!(ok(&c.set(serial, "out.mute", json!([1]), json!(true))));
    let load = c.ask(json!({"t": "preset.load", "serial": serial, "name": "House"}));
    assert!(ok(&load), "{load}");
    for (path, idx, value) in [
        ("out.gain", json!([0]), json!(1.0)),
        ("eq.freq", json!([0, 2]), json!(500.0)),
        ("out.delay", json!([0]), json!(1.0)),
    ] {
        let v = c.set(serial, path, idx, value);
        assert_eq!(code(&v), "installer_lock", "{path}: {v}");
    }
    // The Admin is never locked out.
    assert!(ok(&admin.set(
        serial,
        "eq.freq",
        json!([0, 2]),
        json!(500.0)
    )));
    r.hub.stop();
}

#[test]
fn wp_p2pa_presets_by_name_and_gated_saves() {
    let r = rig("presets");
    let serial = "P2PA-PRESET";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut c = r.client("Venue tablet", Role::Control);

    let unconfirmed =
        admin.ask(json!({"t": "preset.save", "serial": serial, "slot": 2, "name": "Live"}));
    assert_eq!(code(&unconfirmed), "confirmation_required", "{unconfirmed}");
    let denied = c.ask(json!({"t": "preset.save", "serial": serial, "slot": 2, "confirm": true}));
    assert_eq!(code(&denied), "denied");
    assert!(ok(&c.set(serial, "out.gain", json!([1]), json!(-18.0))));
    let saved = admin.ask(
        json!({"t": "preset.save", "serial": serial, "slot": 2, "name": "Live", "confirm": true}),
    );
    assert_eq!(
        (saved["slot"].clone(), saved["name"].clone()),
        (json!(2), json!("Live")),
        "{saved}"
    );

    let list = c.ask(json!({"t": "preset.list", "serial": serial}));
    let slot2 = &list["slots"][2];
    assert_eq!(
        (slot2["name"].clone(), slot2["used"].clone()),
        (json!("Live"), json!(true)),
        "{list}"
    );

    assert!(ok(&c.set(serial, "out.gain", json!([1]), json!(-40.0))));
    let load = c.ask(json!({"t": "preset.load", "serial": serial, "name": "Live"}));
    assert_eq!(
        (load["slot"].clone(), load["name"].clone()),
        (json!(2), json!("Live")),
        "{load}"
    );
    assert_eq!(
        c.get(serial, "out.gain", json!([1]))["value"],
        -18.0,
        "the slot's state"
    );
    let missing = c.ask(json!({"t": "preset.load", "serial": serial, "name": "Nope"}));
    assert_eq!(code(&missing), "not_found");
    let v = r
        .client("Watcher", Role::Viewer)
        .ask(json!({"t": "preset.load", "serial": serial, "slot": 2}));
    assert_eq!(code(&v), "denied");

    // A gated registry path: Admin with "confirm" only.
    assert_eq!(
        code(&c.set(serial, "preset.save", json!([3]), json!(null))),
        "denied"
    );
    assert_eq!(
        code(&admin.set(serial, "preset.save", json!([3]), json!(null))),
        "confirmation_required"
    );
    let gated = admin.ask(
        json!({"t": "param.set", "serial": serial, "path": "preset.save",
        "idx": [3], "confirm": true}),
    );
    assert!(ok(&gated), "{gated}");
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_recall_restores_every_value_and_is_recorded() {
    let r = rig("recall");
    let serial = "P2PA-RECALL";
    r.plug(serial);
    let mut c = r.client("Venue tablet", Role::Control);
    let mut other = r.client("Watcher", Role::Viewer);
    // The state to keep: an image value (bulk) and a name (outside the image).
    assert!(ok(&c.set(serial, "out.gain", json!([0]), json!(-3.0))));
    let app = r.app_core(serial);
    app.write_param("dev.name".into(), vec![], ParamValue::Text("Rack A".into()))
        .unwrap();
    assert_eq!(c.get(serial, "dev.name", json!([]))["value"], "Rack A");
    wait_until("the mirror", || {
        app.process_notifications();
        app.get_state().output_gain_db[0] == -3.0
    });
    let id = r.put_snapshot(&mut c, &app, serial, "Doors");
    let changed = other.next("snapshot.changed");
    assert_eq!(changed["snapshot"].as_str(), Some(id.as_str()), "{changed}");
    assert!(
        ["put", "note"].contains(&changed["change"].as_str().unwrap()),
        "{changed}"
    );
    assert_eq!(changed["hub"], r.hub.hub_id());
    let listed = other.ask(json!({"t": "snapshot.list", "serial": serial}));
    assert_eq!(listed["snapshots"][0]["id"], id.as_str(), "{listed}");

    // Changed since.
    assert!(ok(&c.set(serial, "out.gain", json!([0]), json!(-9.0))));
    assert!(ok(&c.set(serial, "dev.name", json!([]), json!("Other"))));
    let sub = r.hub.bus().subscribe("p2pa-recall", 1024);
    let recall = c.ask(json!({"t": "snapshot.recall", "serial": serial, "name": "Doors"}));
    assert!(ok(&recall), "{recall}");
    assert_eq!(recall["snapshot"], id.as_str());
    assert!(!recall["lines"].as_array().unwrap().is_empty(), "{recall}");
    assert_eq!(
        c.get(serial, "out.gain", json!([0]))["value"],
        -3.0,
        "the image's value"
    );
    assert_eq!(
        c.get(serial, "dev.name", json!([]))["value"],
        "Rack A",
        "outside the image"
    );
    // Recorded for the history, with its lines and who.
    let start = Instant::now();
    let record = loop {
        assert!(start.elapsed() < WAIT, "no recall record");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Event { serial: s, record } = &*m
            && s == serial
        {
            break record.clone();
        }
    };
    assert_eq!(record.kind, EventKind::Recall);
    assert!(!record.lines.is_empty());
    assert_eq!(record.by.as_deref(), Some("Venue tablet"));
    // By id; another device's snapshot or none is not found; a Viewer may not recall.
    assert!(ok(&c.ask(
        json!({"t": "snapshot.recall", "serial": serial, "snapshot": id})
    )));
    let none = c.ask(json!({"t": "snapshot.recall", "serial": serial, "name": "Nope"}));
    assert_eq!(code(&none), "not_found");
    let v = other.ask(json!({"t": "snapshot.recall", "serial": serial, "snapshot": id}));
    assert_eq!(code(&v), "denied");
    // A dropped device gets nothing.
    let handle = r.hub.devices()[0].handle;
    r.hub.detach(handle).unwrap();
    let gone = c.ask(json!({"t": "snapshot.recall", "serial": serial, "snapshot": id}));
    assert_eq!(code(&gone), "offline", "{gone}");
    let gone = c.ask(json!({"t": "param.get", "serial": serial, "path": "vol.master"}));
    assert_eq!(code(&gone), "offline", "{gone}");
    app.disconnect();
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_recall_is_refused_for_control_on_a_limited_device_and_held_for_admin() {
    let r = rig("recall-limits");
    let serial = "P2PA-RLIM";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut c = r.client("Venue tablet", Role::Control);
    assert!(ok(&admin.set(serial, "out.gain", json!([0]), json!(6.0))));
    let app = r.app_core(serial);
    wait_until("the mirror", || {
        app.process_notifications();
        app.get_state().output_gain_db[0] == 6.0
    });
    let id = r.put_snapshot(&mut admin, &app, serial, "Loud");
    assert!(ok(&admin.set(serial, "out.gain", json!([0]), json!(0.0))));
    assert!(ok(&admin.ask(
        json!({"t": "limits.set", "handle": handle, "profile": gain_profile()})
    )));

    let refused = c.ask(json!({"t": "snapshot.recall", "serial": serial, "snapshot": id}));
    assert_eq!(code(&refused), "image_refused", "{refused}");
    assert_eq!(
        c.get(serial, "out.gain", json!([0]))["value"],
        0.0,
        "nothing written"
    );

    let accepted = admin.ask(json!({"t": "snapshot.recall", "serial": serial, "snapshot": id}));
    assert!(ok(&accepted), "{accepted}");
    // Then held to the profile after the fact.
    wait_until("the hold", || {
        c.get(serial, "out.gain", json!([0]))["value"]
            .as_f64()
            .unwrap()
            <= 3.0
    });
    app.disconnect();
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_guarded_recall_reverts_on_timeout_and_a_pending_guard_is_a_conflict() {
    let r = rig("guard");
    let serial = "P2PA-GUARD";
    r.plug(serial);
    let mut c = r.client("Venue tablet", Role::Control);
    let mut b = r.client("Second tablet", Role::Control);
    assert!(ok(&c.set(serial, "out.gain", json!([0]), json!(-6.0))));
    let app = r.app_core(serial);
    wait_until("the mirror", || {
        app.process_notifications();
        app.get_state().output_gain_db[0] == -6.0
    });
    let id = r.put_snapshot(&mut c, &app, serial, "Quiet");
    assert!(ok(&c.set(serial, "out.gain", json!([0]), json!(-1.0))));

    let recall = c.ask(
        json!({"t": "snapshot.recall", "serial": serial, "snapshot": id,
        "guard": true, "timeout_ms": 1500}),
    );
    assert!(ok(&recall), "{recall}");
    assert!(recall["guard"].as_u64().is_some(), "{recall}");
    assert_eq!(
        c.get(serial, "out.gain", json!([0]))["value"],
        -6.0,
        "recalled"
    );
    // Another session's action while the guard is pending: conflict, nothing sent.
    let busy = b.set(serial, "out.gain", json!([0]), json!(-2.0));
    assert_eq!(code(&busy), "conflict", "{busy}");
    // No confirm: the hub writes the captured state back.
    let ended = c.next("apply.ended");
    assert_eq!(ended["result"], "reverted", "{ended}");
    wait_until("the revert", || {
        c.get(serial, "out.gain", json!([0]))["value"] == -1.0
    });
    // Over: actions go again.
    assert!(ok(&b.set(serial, "out.gain", json!([0]), json!(-2.0))));
    app.disconnect();
    r.hub.stop();
}

#[test]
fn wp_p2pa_the_core_opened_for_a_client_closes_when_the_client_goes() {
    let r = rig("close");
    let serial = "P2PA-CLOSE";
    r.plug(serial);
    let sub = r.hub.bus().subscribe("p2pa-close", 1024);
    let mut c = r.client("Venue tablet", Role::Control);
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-22.0))));
    let (child, _) = next_write(&sub, serial);
    assert!(!r.events.closed(child));
    drop(c);
    wait_until("the child session to close", || r.events.closed(child));
    r.hub.stop();
}

#[test]
fn wp_p2pa_an_idle_core_closes_after_its_minute() {
    let clock = Arc::new(TestClock::new());
    let r = rig_with("idle", Some(Arc::clone(&clock) as Arc<dyn HubClock>));
    let serial = "P2PA-IDLE";
    r.plug(serial);
    let sub = r.hub.bus().subscribe("p2pa-idle", 1024);
    let mut c = r.client("Venue tablet", Role::Control);
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-22.0))));
    let (child, _) = next_write(&sub, serial);
    clock.advance(Duration::from_secs(30));
    r.hub.wake_services();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!r.events.closed(child), "not idle yet");
    clock.advance(dspi_hub::api::IDLE_CLOSE);
    r.hub.wake_services();
    wait_until("the idle close", || r.events.closed(child));
    // The next action opens a new one.
    assert!(ok(&c.set(serial, "vol.master", json!([]), json!(-21.0))));
    let (again, _) = next_write(&sub, serial);
    assert_ne!(again, child);
    r.hub.stop();
}

// ── The child acts for its parent's device lock and PIN unlock ──────────────

#[test]
fn wp_p2pa_the_callers_own_device_lock_lets_its_api_writes_through() {
    let r = rig("acts-lock");
    let serial = "P2PA-ACTS";
    let handle = r.plug(serial);
    let mut c = r.client("Venue tablet", Role::Control);
    let mut b = r.client("Second tablet", Role::Control);
    // Opened before the lock, and after: both pass.
    assert!(ok(&c.set(serial, "out.gain", json!([0]), json!(-4.0))));
    let lock = c.ask(json!({"t": "lock.acquire", "handle": handle, "timeout_ms": 20_000}));
    assert!(ok(&lock), "{lock}");
    let own = c.set(serial, "out.gain", json!([0]), json!(-5.0));
    assert_eq!(own["value"], -5.0, "the caller's own lock: {own}");
    let load = c.ask(json!({"t": "preset.load", "serial": serial, "slot": 0}));
    assert!(
        ok(&load),
        "a flow that takes the lock itself, nested: {load}"
    );
    assert!(
        ok(&c.set(serial, "out.gain", json!([0]), json!(-5.5))),
        "the lock is still the caller's"
    );
    // Another session meanwhile: locked.
    let other = b.set(serial, "out.gain", json!([0]), json!(-6.0));
    assert_eq!(code(&other), "locked", "{other}");
    assert_eq!(
        code(&b.ask(json!({"t": "lock.acquire", "handle": handle}))),
        "locked"
    );
    // And the other way round.
    assert!(ok(&c.ask(json!({"t": "lock.release", "handle": handle}))));
    assert!(ok(&b.ask(
        json!({"t": "lock.acquire", "handle": handle, "timeout_ms": 20_000})
    )));
    let refused = c.set(serial, "out.gain", json!([0]), json!(-7.0));
    assert_eq!(code(&refused), "locked", "{refused}");
    assert!(ok(&b.set(serial, "out.gain", json!([0]), json!(-8.0))));
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_pin_unlocked_control_caller_writes_a_covered_path_through_the_api() {
    let r = rig("acts-pin");
    let serial = "P2PA-ACTSPIN";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq", "routing"]);
    assert!(ok(&admin.ask(
        json!({"t": "limits.set", "handle": handle, "profile": p})
    )));
    assert!(ok(&admin.ask(
        json!({"t": "limits.pin", "handle": handle, "pin": "2468"})
    )));
    let mut c = r.client("Venue tablet", Role::Control);
    let mut d = r.client("Second tablet", Role::Control);
    let locked = c.set(serial, "eq.freq", json!([0, 2]), json!(700.0));
    assert_eq!(code(&locked), "installer_lock", "{locked}");
    let unlock = c.ask(json!({"t": "limits.unlock", "handle": handle, "pin": "2468"}));
    assert!(ok(&unlock), "{unlock}");
    let set = c.set(serial, "eq.freq", json!([0, 2]), json!(700.0));
    assert_eq!(set["value"], 700.0, "unlocked by the caller's PIN: {set}");
    // Still held to the limits.
    assert_eq!(
        code(&c.set(serial, "out.gain", json!([0]), json!(6.0))),
        "limit_exceeded"
    );
    // A caller that did not unlock stays in the lock's scope.
    let other = d.set(serial, "eq.freq", json!([0, 2]), json!(800.0));
    assert_eq!(code(&other), "installer_lock", "{other}");
    r.hub.stop();
}

#[test]
fn wp_p2pa_a_child_never_keeps_its_parents_lock_or_unlock_after_the_parent_goes() {
    let r = rig("acts-gone");
    let serial = "P2PA-ACTSGONE";
    let handle = r.plug(serial);
    let h = r.hub.handle();
    let gain = |db: f32| db.to_le_bytes().to_vec();

    // The device lock.
    let parent = h.open_session(Role::Control, None).unwrap();
    let child = h.open_child(&parent).unwrap();
    let mut t = h.open_as(serial, &child, false).unwrap();
    h.lock_request(&parent, handle, Duration::from_secs(20))
        .unwrap();
    t.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &gain(-3.0))
        .unwrap();
    h.close_session(parent.id());
    assert!(child.parent_gone(&h));
    let next = h.open_session(Role::Control, None).unwrap();
    h.lock_request(&next, handle, Duration::from_secs(20))
        .expect("the parent's lock went with it; the child did not take it");
    assert!(matches!(
        t.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &gain(-4.0)),
        Err(TransportError::Locked)
    ));
    h.lock_release(&next, handle);
    drop(t);
    h.close_session(child.id());

    // The PIN unlock.
    let admin = h.open_session(Role::Admin, None).unwrap();
    let ask = |s: &dspi_hub::Session, m: Value| -> Value {
        serde_json::from_str(&dspi_hub::limits::handle_message(&h, s, &m).unwrap()).unwrap()
    };
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq"]);
    assert!(ok(&ask(
        &admin,
        json!({"t": "limits.set", "id": 1, "handle": handle, "profile": p})
    )));
    assert!(ok(&ask(
        &admin,
        json!({"t": "limits.pin", "id": 2, "handle": handle, "pin": "2468"})
    )));
    let parent = h.open_session(Role::Control, None).unwrap();
    let child = h.open_child(&parent).unwrap();
    let mut t = h.open_as(serial, &child, false).unwrap();
    assert!(ok(&ask(
        &parent,
        json!({"t": "limits.unlock", "id": 3, "handle": handle, "pin": "2468"})
    )));
    t.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &gain(1.0))
        .expect("the parent's unlock is the child's");
    h.close_session(parent.id());
    let refused = t
        .control_out(op::REQ_SET_OUTPUT_GAIN, 0, &gain(2.0))
        .unwrap_err();
    assert_eq!(
        refused.hub_refusal().map(|(k, _)| k),
        Some("installer_lock"),
        "{refused:?}"
    );
    r.hub.stop();
}
