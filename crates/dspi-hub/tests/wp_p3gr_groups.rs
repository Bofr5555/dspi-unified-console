//! WP P3GR's done criteria through the real server on the demo device: `group.*` over a
//! WebSocket of a paired client with its role, members checked against the inventory, a
//! forgotten device leaving its groups, the bounds, a torn file, and a schedule rule on
//! `group:<id>` run on every member (feature `api`: the schedule runs through the
//! executor).
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role, ServiceConfig, TestClock};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
/// Monday 2027-01-04 12:00:00 UTC.
const MONDAY_NOON: u64 = 1_799_064_000;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3gr").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Rig {
    hub: Hub,
    clock: Arc<TestClock>,
}

fn start(dir: &Path, clock: &Arc<TestClock>, inventory: bool) -> Hub {
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
            clock: Arc::clone(clock) as Arc<dyn dspi_hub::HubClock>,
            inventory,
            groups: true,
            #[cfg(feature = "api")]
            schedule: Some(dspi_hub::ScheduleConfig {
                zone: Some("UTC0".into()),
                not_before_s: None,
            }),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p3gr")
    })
    .unwrap()
}

fn rig_in(dir: &Path) -> Rig {
    let clock = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(MONDAY_NOON)));
    let hub = start(dir, &clock, true);
    Rig { hub, clock }
}

fn rig(tag: &str) -> Rig {
    rig_in(&scratch(tag))
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
            "client": {"name": name, "app": "wp_p3gr", "version": "0"}}));
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

    /// Move the hub's clock on and let its timer look.
    fn advance(&self, by: Duration) {
        self.clock.advance(by);
        self.hub.handle().wake_services();
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
        // Pings and binary frames do not count: a hub that only sends those still times out.
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no text frame in time");
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    /// The next message of type `t` that `pick` takes (kept ones first).
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

    /// `group.set`, which must go through: the group.
    fn set(&mut self, body: Value) -> Value {
        let body_is_new = body.get("id").is_none();
        let v = self.ask(gset(body));
        assert!(ok(&v), "{v}");
        assert_eq!(v["v"], 1);
        assert_eq!(v["created"], body_is_new, "{v}");
        v["group"].clone()
    }
}

/// `group.set` of `group` (the group's object, hub-api.md 19.2).
fn gset(group: Value) -> Value {
    json!({"t": "group.set", "group": group})
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

#[test]
fn wp_p3gr_an_admin_sets_and_deletes_control_and_viewer_are_refused_viewer_lists() {
    let r = rig("roles");
    let (a, b) = ("P3GR-A", "P3GR-B");
    r.plug(a);
    r.plug(b);
    let mut admin = r.client("Installer", Role::Admin);
    let mut control = r.client("Venue tablet", Role::Control);
    let mut viewer = r.client("Watcher", Role::Viewer);
    assert!(
        viewer.caps.iter().any(|c| c == "groups"),
        "{:?}",
        viewer.caps
    );

    let g = admin.set(json!({"name": "Main hall", "note": "left and right", "members": [a, b]}));
    assert_eq!(
        g,
        json!({"id": "g1", "name": "Main hall", "note": "left and right", "members": [a, b]})
    );
    let changed = viewer.next("group.changed", |_| true);
    assert_eq!(changed["v"], 1);
    assert_eq!(changed["change"], "set");
    assert_eq!(changed["group"], g);
    assert!(
        changed.get("id").is_none(),
        "an event has no request id: {changed}"
    );
    assert_eq!(changed["hub"], r.hub.handle().hub_id());

    for c in [&mut control, &mut viewer] {
        for msg in [
            gset(json!({"name": "Mine", "members": [a]})),
            gset(json!({"id": "g1", "name": "Mine", "members": [a]})),
            json!({"t": "group.delete", "group": "g1"}),
        ] {
            let v = c.ask(msg.clone());
            assert_eq!(code(&v), "denied", "{msg}: {v}");
            assert_eq!(v["v"], 1);
        }
        let list = c.ask(json!({"t": "group.list"}));
        assert!(ok(&list), "{list}");
        assert_eq!(list["groups"], json!([g]));
        let one = c.ask(json!({"t": "group.list", "group": "g1"}));
        assert_eq!(one["groups"], json!([g]), "{one}");
        assert_eq!(
            code(&c.ask(json!({"t": "group.list", "group": "g7"}))),
            "not_found"
        );
    }

    // A member the inventory does not hold, and an unknown group.
    let v = admin.ask(gset(json!({"name": "Bar", "members": [a, "P3GR-NEVER"]})));
    assert_eq!(code(&v), "not_found", "{v}");
    assert!(v["msg"].as_str().unwrap().contains("P3GR-NEVER"), "{v}");
    let v = admin.ask(gset(json!({"id": "g9", "name": "Bar", "members": []})));
    assert_eq!(code(&v), "not_found", "{v}");
    assert_eq!(
        code(&admin.ask(json!({"t": "group.delete", "group": "g9"}))),
        "not_found"
    );

    // A group is no member (no nesting), and a request without the group's object.
    let v = admin.ask(gset(json!({"name": "Bar", "members": ["group:g1"]})));
    assert_eq!(code(&v), "bad_request", "{v}");
    let v = admin.ask(json!({"t": "group.set", "name": "Bar", "members": []}));
    assert_eq!(code(&v), "bad_request", "{v}");

    // Replaced whole, then deleted.
    let replaced = admin.ask(gset(json!({"id": "g1", "name": "Hall", "members": [b]})));
    assert_eq!(replaced["created"], false, "{replaced}");
    let g = replaced["group"].clone();
    assert_eq!(
        g,
        json!({"id": "g1", "name": "Hall", "note": null, "members": [b]})
    );
    let del = admin.ask(json!({"t": "group.delete", "group": "g1"}));
    assert!(ok(&del), "{del}");
    assert_eq!(del["group"], "g1");
    let gone = viewer.next("group.changed", |v| v["change"] == "delete");
    assert_eq!(gone["group"], g, "the group as it was");
    assert!(
        viewer.ask(json!({"t": "group.list"}))["groups"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // A new group never takes a deleted one's id.
    assert_eq!(
        admin.set(json!({"name": "Foyer", "members": []}))["id"],
        "g2"
    );
    r.hub.stop();
}

#[test]
fn wp_p3gr_the_65th_group_is_a_conflict_and_the_65th_member_bad_request() {
    let r = rig("bounds");
    r.plug("P3GR-ONE");
    let mut admin = r.client("Installer", Role::Admin);
    for i in 1..=64 {
        let g = admin.set(json!({"name": format!("Group {i}"), "members": ["P3GR-ONE"]}));
        assert_eq!(g["id"], format!("g{i}"));
    }
    let v = admin.ask(gset(json!({"name": "One too many", "members": []})));
    assert_eq!(code(&v), "conflict", "{v}");
    let members: Vec<String> = (0..65).map(|i| format!("P3GR-M{i:02}")).collect();
    let v = admin.ask(gset(json!({"id": "g1", "name": "Big", "members": members})));
    assert_eq!(code(&v), "bad_request", "{v}");
    let v = admin.ask(gset(json!({"name": "x".repeat(49), "members": []})));
    assert_eq!(code(&v), "bad_request", "{v}");
    let v = admin.ask(gset(
        json!({"id": "g1", "name": "N", "note": "n".repeat(257),
        "members": []}),
    ));
    assert_eq!(code(&v), "bad_request", "{v}");
    assert_eq!(
        admin.ask(json!({"t": "group.list"}))["groups"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    r.hub.stop();
}

#[test]
fn wp_p3gr_a_forgotten_device_leaves_its_groups_and_group_changed_says_so() {
    let r = rig("forget");
    let (stays, goes) = ("P3GR-STAYS", "P3GR-GOES");
    r.plug(stays);
    let handle = r.plug(goes);
    let mut admin = r.client("Installer", Role::Admin);
    let mut viewer = r.client("Watcher", Role::Viewer);
    admin.set(json!({"name": "Hall", "members": [stays, goes]}));
    admin.set(json!({"name": "Only it", "members": [goes]}));
    admin.set(json!({"name": "Other", "members": [stays]}));
    for _ in 0..3 {
        viewer.next("group.changed", |_| true);
    }

    r.hub.handle().detach(handle).unwrap();
    // The registry has it offline (in its grace) before an Admin may forget it.
    let start = Instant::now();
    let forget = loop {
        let v = admin.ask(json!({"t": "inventory.forget", "serial": goes}));
        if ok(&v) || start.elapsed() > WAIT {
            break v;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(ok(&forget), "{forget}");
    // The groups hear of it on the bus, on their next tick.
    r.advance(Duration::from_secs(2));
    let hall = viewer.next("group.changed", |v| v["group"]["id"] == "g1");
    assert_eq!(hall["change"], "forgot");
    assert_eq!(hall["group"]["members"], json!([stays]), "{hall}");
    let only = viewer.next("group.changed", |v| v["group"]["id"] == "g2");
    assert_eq!(only["change"], "forgot");
    assert_eq!(only["group"]["members"], json!([]), "{only}");
    let list = viewer.ask(json!({"t": "group.list"}));
    assert_eq!(list["groups"][0]["members"], json!([stays]));
    assert_eq!(list["groups"][1]["members"], json!([]));
    assert_eq!(list["groups"][2]["members"], json!([stays]));
    // No `group.changed` for the group it was not in.
    assert!(
        !viewer.seen.iter().any(|v| v["t"] == "group.changed"
            && v["change"] == "forgot"
            && v["group"]["id"] == "g3")
    );
    // A forgotten serial is no member any more.
    let v = admin.ask(gset(
        json!({"id": "g2", "name": "Only it", "members": [goes]}),
    ));
    assert_eq!(code(&v), "not_found", "{v}");
    r.hub.stop();
}

// -- WP P3GP -- the P3-A review: a record the inventory drops past its cap is forgotten.
#[test]
fn wp_p3gp_a_device_dropped_past_the_inventory_cap_leaves_its_groups() {
    let dir = scratch("cap");
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();
    // A full inventory (1024 records, all offline); OLD-0000 was seen longest ago.
    let cap = 1024;
    let mut lines: Vec<String> = Vec::new();
    for i in 0..cap {
        lines.push(
            json!({"serial": format!("P3GP-OLD-{i:04}"), "first_seen_s": 1000 + i,
                "last_seen_s": 1000 + i})
            .to_string(),
        );
    }
    std::fs::write(
        store.join("inventory.json"),
        format!("{{\"v\":1,\"devices\":[\n{}\n]}}\n", lines.join(",\n")),
    )
    .unwrap();
    std::fs::write(
        store.join("groups.json"),
        "{\"v\":1,\"next_id\":3,\"groups\":[\n\
         {\"id\":\"g1\",\"name\":\"Hall\",\"members\":[\"P3GP-OLD-0000\",\"P3GP-OLD-0005\"]},\n\
         {\"id\":\"g2\",\"name\":\"Bar\",\"members\":[\"P3GP-OLD-0005\"]}\n]}\n",
    )
    .unwrap();
    let r = rig_in(&dir);
    let mut viewer = r.client("Watcher", Role::Viewer);
    let before = viewer.ask(json!({"t": "group.list", "group": "g1"}));
    assert_eq!(
        before["groups"][0]["members"],
        json!(["P3GP-OLD-0000", "P3GP-OLD-0005"]),
        "{before}"
    );

    // A new device: the inventory makes room by dropping OLD-0000.
    r.plug("P3GP-NEW");
    let start = Instant::now();
    let changed = loop {
        assert!(
            start.elapsed() < WAIT,
            "no group.changed for the dropped device"
        );
        r.advance(Duration::from_secs(2));
        let list = viewer.ask(json!({"t": "group.list", "group": "g1"}));
        if list["groups"][0]["members"] == json!(["P3GP-OLD-0005"]) {
            break viewer.next("group.changed", |v| v["group"]["id"] == "g1");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(changed["change"], "forgot", "{changed}");
    assert_eq!(changed["group"]["members"], json!(["P3GP-OLD-0005"]));
    let gone = viewer.ask(json!({"t": "inventory.list", "serial": "P3GP-OLD-0000"}));
    assert_eq!(code(&gone), "not_found", "{gone}");
    // The group it was not in is untouched.
    let bar = viewer.ask(json!({"t": "group.list", "group": "g2"}));
    assert_eq!(bar["groups"][0]["members"], json!(["P3GP-OLD-0005"]));
    assert!(
        !viewer
            .seen
            .iter()
            .any(|v| v["t"] == "group.changed" && v["group"]["id"] == "g2")
    );
    r.hub.stop();
}

#[test]
fn wp_p3gr_groups_survive_a_restart_and_a_torn_file_reads_what_it_can() {
    let dir = scratch("file");
    {
        let r = rig_in(&dir);
        r.plug("P3GR-F1");
        let mut admin = r.client("Installer", Role::Admin);
        admin.set(json!({"name": "Hall", "members": ["P3GR-F1"]}));
        admin.set(json!({"name": "Bar", "members": []}));
        admin.set(json!({"name": "Foyer", "members": ["P3GR-F1"]}));
        r.hub.stop();
    }
    let file = dir.join("store").join("groups.json");
    let text = std::fs::read_to_string(&file).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "groups.json is the hub's alone");
    }
    // Torn in the last group's line, as a power cut mid-write could leave it.
    std::fs::write(&file, &text[..text.find("Foyer").unwrap()]).unwrap();
    let r = rig_in(&dir);
    let mut viewer = r.client("Watcher", Role::Viewer);
    let list = viewer.ask(json!({"t": "group.list"}));
    let names: Vec<&str> = list["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Hall", "Bar"], "{list}");
    let aside = std::fs::read_dir(dir.join("store"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("groups.json.bad-")
        });
    assert!(aside, "the torn file is kept aside");
    // The hub runs on: g3 was given once, so a new group is g4.
    r.plug("P3GR-F1");
    let mut admin = r.client("Installer", Role::Admin);
    assert_eq!(
        admin.set(json!({"name": "Foyer", "members": []}))["id"],
        "g4"
    );
    r.hub.stop();
}

#[test]
fn wp_p3gr_without_the_inventory_members_are_the_devices_the_hub_knows() {
    let dir = scratch("no-inventory");
    let clock = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(MONDAY_NOON)));
    let r = Rig {
        hub: start(&dir, &clock, false),
        clock,
    };
    r.plug("P3GR-REG");
    let mut admin = r.client("Installer", Role::Admin);
    admin.set(json!({"name": "Hall", "members": ["P3GR-REG"]}));
    let v = admin.ask(gset(json!({"name": "Bar", "members": ["P3GR-NEVER"]})));
    assert_eq!(code(&v), "not_found", "{v}");
    r.hub.stop();
}

#[cfg(feature = "api")]
mod schedule {
    use super::*;

    fn get(ws: &mut Ws, serial: &str, path: &str) -> Value {
        let v = ws.ask(json!({"t": "param.get", "serial": serial, "path": path}));
        assert_eq!(v["t"], "ok", "{v}");
        v["value"].clone()
    }

    fn rule(name: &str, targets: Value, actions: Value) -> Value {
        json!({"t": "schedule.set", "rule": {"name": name,
            "when": {"days": ["mon"], "time": "18:30"},
            "actions": actions, "targets": targets}})
    }

    #[test]
    fn wp_p3gr_a_rule_on_a_group_runs_on_every_member_and_an_unknown_group_is_one_row() {
        let r = rig("schedule");
        let (a, b, c) = ("P3GR-SA", "P3GR-SB", "P3GR-SC");
        for s in [a, b, c] {
            r.plug(s);
        }
        let mut admin = r.client("Installer", Role::Admin);
        admin.set(json!({"name": "Hall", "members": [a, b]}));
        let set = admin.ask(rule(
            "Doors",
            json!(["group:g1", c, "group:g9"]),
            json!([{"t": "param.set", "path": "vol.master", "value": -21.0}]),
        ));
        assert!(ok(&set), "{set}");
        let id = set["rule"]["id"].as_str().unwrap().to_owned();
        assert_eq!(set["rule"]["targets"], json!(["group:g1", c, "group:g9"]));

        let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
        assert!(ok(&ran), "{ran}");
        assert_eq!(
            ran["results"],
            json!([
                {"serial": a, "action": "0:param.set", "ok": true},
                {"serial": b, "action": "0:param.set", "ok": true},
                {"serial": c, "action": "0:param.set", "ok": true},
                {"serial": "group:g9", "action": "0:param.set", "ok": false,
                 "code": "not_found", "msg": "no such group"},
            ])
        );
        for s in [a, b, c] {
            assert_eq!(get(&mut admin, s, "vol.master"), -21.0, "{s}");
        }
        // The members of the moment: a group edited after the rule was set runs on its new
        // members, and a device in the group and named alone runs once.
        admin.set(json!({"id": "g1", "name": "Hall", "members": [c, a]}));
        let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
        let serials: Vec<&str> = ran["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["serial"].as_str().unwrap())
            .collect();
        assert_eq!(serials, [c, a, "group:g9"], "{ran}");
        r.hub.stop();
    }

    #[test]
    fn wp_p3gr_a_run_answers_at_most_256_rows_the_last_saying_truncated() {
        let r = rig("truncated");
        // 17 devices the inventory knows; all but one offline (each fails at once).
        let serials: Vec<String> = (0..17).map(|i| format!("P3GR-T{i:02}")).collect();
        let handles: Vec<u8> = serials.iter().map(|s| r.plug(s)).collect();
        let mut admin = r.client("Installer", Role::Admin);
        admin.set(json!({"name": "Everything", "members": serials}));
        for h in handles.iter().skip(1) {
            r.hub.handle().detach(*h).unwrap();
        }
        // 16 actions on 17 members would be 272 rows.
        let actions: Vec<Value> = (0..16)
            .map(|i| json!({"t": "param.set", "path": "vol.master", "value": -30.0 - f64::from(i)}))
            .collect();
        let set = admin.ask(rule("Big", json!(["group:g1"]), json!(actions)));
        assert!(ok(&set), "{set}");
        let id = set["rule"]["id"].as_str().unwrap().to_owned();
        let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
        assert!(ok(&ran), "{ran}");
        let rows = ran["results"].as_array().unwrap();
        assert_eq!(rows.len(), 256);
        let last = &rows[255];
        assert_eq!(last["ok"], false);
        assert_eq!(last["code"], "bad_request");
        assert!(
            last["msg"].as_str().unwrap().starts_with("truncated"),
            "{last}"
        );
        assert_eq!(last["action"], "15:param.set");
        // The 255 rows before it ran: 15 actions on all 17, and the 16th on 0 of them.
        assert_eq!(rows[254]["action"], "14:param.set");
        assert!(rows[..255].iter().all(|r| r["code"] != "bad_request"));
        // The online member got the 15th action, not the 16th (which never ran).
        assert_eq!(get(&mut admin, &serials[0], "vol.master"), -44.0);
        r.hub.stop();
    }
}
