//! WP P2SC's done criteria through the real server on the demo device: `schedule.*` over
//! a WebSocket of a paired client with its role, the runs as the principal
//! `Schedule: <name>` (Admin, a remote-range session) through P2PA's executor, the hub's
//! test clock driving a run and a jump, the unset clock, `schedule.ran` and the history.
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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dspi_hub::history::HistoryConfig;
use dspi_hub::session::FIRST_REMOTE_SESSION_ID;
use dspi_hub::{
    AccessMode, BusMessage, Hub, HubConfig, NetworkConfig, Role, ScheduleConfig, ServiceConfig,
    Subscription, TestClock,
};
use dspi_session::events::{EventKind, EventRecord};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
/// Monday 2027-01-04 12:00:00 UTC.
const MONDAY_NOON: u64 = 1_799_064_000;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/wp-P2SC/tmp, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2sc").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wall(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

struct Rig {
    hub: Hub,
    clock: Arc<TestClock>,
    bus: Subscription,
}

fn start(dir: &Path, clock: &Arc<TestClock>) -> Hub {
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
            history: Some(HistoryConfig::default()),
            schedule: Some(ScheduleConfig {
                zone: Some("UTC0".into()),
                not_before_s: None,
            }),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p2sc")
    })
    .unwrap()
}

fn rig_at(dir: &Path, clock: Arc<TestClock>) -> Rig {
    let hub = start(dir, &clock);
    let bus = hub.handle().bus().subscribe("p2sc-test", 4096);
    Rig { hub, clock, bus }
}

fn rig(tag: &str) -> Rig {
    rig_at(&scratch(tag), Arc::new(TestClock::at(wall(MONDAY_NOON))))
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
            "client": {"name": name, "app": "wp_p2sc", "version": "0"}}));
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

    /// Move the hub's clock on (both clocks) and let its timer look.
    fn advance(&self, by: Duration) {
        self.clock.advance(by);
        self.hub.handle().wake_services();
    }

    /// Set the wall clock alone (a jump), then let the timer's next look come (it looks
    /// at least every `TICK` of monotonic time).
    fn set_wall(&self, secs: u64) {
        self.clock.set_wall(wall(secs));
        self.advance(dspi_hub::schedule::TICK);
    }

    /// The next bus message `pick` takes.
    fn next_on_bus<T>(&self, what: &str, mut pick: impl FnMut(&BusMessage) -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {what} on the bus");
            if let Some(m) = self.bus.recv_timeout(Duration::from_millis(50))
                && let Some(t) = pick(&m)
            {
                return t;
            }
        }
    }

    /// The next `schedule` record the history wrote (it has an id) for `serial`.
    fn schedule_record(&self, serial: &str) -> EventRecord {
        self.next_on_bus("a schedule record", |m| match m {
            BusMessage::Event { serial: s, record }
                if s == serial && record.id != 0 && record.kind == EventKind::Schedule =>
            {
                Some(record.clone())
            }
            _ => None,
        })
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

    fn get(&mut self, serial: &str, path: &str) -> Value {
        let v = self.ask(json!({"t": "param.get", "serial": serial, "path": path}));
        assert_eq!(v["t"], "ok", "{v}");
        v["value"].clone()
    }

    /// `schedule.set` of a new rule: its id.
    fn add(&mut self, rule: Value) -> String {
        let v = self.ask(json!({"t": "schedule.set", "rule": rule}));
        assert!(ok(&v), "{v}");
        v["rule"]["id"].as_str().unwrap().to_owned()
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn volume_rule(name: &str, targets: &[&str], db: f64) -> Value {
    json!({"name": name, "when": {"days": ["mon"], "time": "18:30"},
           "actions": [{"t": "param.set", "path": "vol.master", "value": db},
                       {"t": "param.set", "path": "vol.mute", "value": true}],
           "targets": targets})
}

#[test]
fn wp_p2sc_only_an_admin_edits_or_runs_and_every_role_lists() {
    let r = rig("roles");
    let serial = "P2SC-ROLES";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut control = r.client("Venue tablet", Role::Control);
    let mut viewer = r.client("Watcher", Role::Viewer);
    assert!(
        viewer.caps.iter().any(|c| c == "schedule"),
        "{:?}",
        viewer.caps
    );

    let id = admin.add(volume_rule("Doors", &[serial], -12.0));
    assert_eq!(id, "r1");
    for c in [&mut control, &mut viewer] {
        for msg in [
            json!({"t": "schedule.set", "rule": volume_rule("Mine", &[serial], -3.0)}),
            json!({"t": "schedule.delete", "rule": id}),
            json!({"t": "schedule.run_now", "rule": id}),
        ] {
            let v = c.ask(msg.clone());
            assert_eq!(code(&v), "denied", "{msg}: {v}");
            assert_eq!(v["v"], 1);
        }
        let list = c.ask(json!({"t": "schedule.list"}));
        assert!(ok(&list), "{list}");
        assert_eq!(list["rules"].as_array().unwrap().len(), 1);
        let rule = &list["rules"][0];
        assert_eq!(rule["id"], "r1");
        assert_eq!(rule["when"], json!({"days": ["mon"], "time": "18:30"}));
        assert_eq!(rule["next_run"], "2027-01-04T18:30:00Z", "{rule}");
        assert_eq!(rule["last"], Value::Null);
        assert_eq!(list["zone"], "UTC0");
        assert_eq!(list["local_time"], "2027-01-04T12:00");
        assert_eq!(list["clock_unset"], false);
    }
    // Admin edits: an unknown rule, a malformed group target, a rule changed and deleted.
    let mut changed = volume_rule("Doors", &[serial], -14.0);
    changed["id"] = json!("r9");
    assert_eq!(
        code(&admin.ask(json!({"t": "schedule.set", "rule": changed.clone()}))),
        "not_found"
    );
    changed["id"] = json!("r1");
    // -- WP P3GR -- `group:<id>` is resolved at run time now; one with no id is refused.
    changed["targets"] = json!(["group:"]);
    assert_eq!(
        code(&admin.ask(json!({"t": "schedule.set", "rule": changed.clone()}))),
        "bad_request"
    );
    changed["targets"] = json!([serial]);
    let set = admin.ask(json!({"t": "schedule.set", "rule": changed}));
    assert_eq!(set["rule"]["actions"][0]["value"], -14.0, "{set}");
    let del = admin.ask(json!({"t": "schedule.delete", "rule": "r1"}));
    assert_eq!(del["rule"], "r1", "{del}");
    assert_eq!(
        code(&admin.ask(json!({"t": "schedule.run_now", "rule": "r1"}))),
        "not_found"
    );
    assert!(
        admin.ask(json!({"t": "schedule.list"}))["rules"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    r.hub.stop();
}

#[test]
fn wp_p2sc_run_now_writes_as_the_schedule_with_the_admin_in_by() {
    let r = rig("run-now");
    let serial = "P2SC-RUN";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut watcher = r.client("Watcher", Role::Viewer);
    let id = admin.add(volume_rule("Doors", &[serial], -20.0));

    let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
    assert!(ok(&ran), "{ran}");
    assert_eq!(ran["v"], 1);
    assert_eq!(
        (ran["rule"].as_str(), ran["name"].as_str()),
        (Some("r1"), Some("Doors"))
    );
    assert_eq!(ran["by"], "Installer");
    assert_eq!(ran["at"], "2027-01-04T12:00:00Z");
    assert_eq!(
        ran["results"],
        json!([{"serial": serial, "action": "0:param.set", "ok": true},
               {"serial": serial, "action": "1:param.set", "ok": true}])
    );
    assert_eq!(watcher.get(serial, "vol.master"), -20.0);
    assert_eq!(watcher.get(serial, "vol.mute"), true);

    // The write was the schedule's: a session from the remote range, named for the rule.
    let (session, by) = r.next_on_bus("the write", |m| match m {
        BusMessage::Notification {
            serial: s,
            session,
            by,
            ..
        } if s == serial && *session != 0 => Some((*session, by.clone())),
        _ => None,
    });
    assert!(session >= FIRST_REMOTE_SESSION_ID, "{session}");
    assert_eq!(by.as_deref(), Some("Schedule: Doors"));
    // Everyone hears of the run; the bus carries it with `by`; the history records it.
    let event = watcher.next("schedule.ran");
    assert_eq!(event["v"], 1);
    assert_eq!(event["rule"], "r1");
    assert_eq!(event["by"], "Installer");
    assert!(
        event["hub"].as_str().is_some_and(|h| !h.is_empty()),
        "{event}"
    );
    assert_eq!(event["results"], ran["results"]);
    let (by, results) = r.next_on_bus("ScheduleRan", |m| match m {
        BusMessage::ScheduleRan {
            rule, by, results, ..
        } if rule == "r1" => Some((by.clone(), results.clone())),
        _ => None,
    });
    assert_eq!(by.as_deref(), Some("Installer"));
    assert_eq!(results.len(), 2);
    let record = r.schedule_record(serial);
    assert_eq!(record.by.as_deref(), Some("Installer"));
    assert_eq!(record.source.as_deref(), Some("r1"));
    assert_eq!(record.lines, ["0:param.set: ok", "1:param.set: ok"]);
    // `last` follows.
    let list = watcher.ask(json!({"t": "schedule.list"}));
    assert_eq!(
        list["rules"][0]["last"],
        json!({"at": "2027-01-04T12:00:00Z", "ok": true})
    );
    r.hub.stop();
}

#[test]
fn wp_p2sc_an_offline_target_fails_at_once_and_a_pending_guard_is_a_conflict() {
    let r = rig("offline-guard");
    let (here, gone) = ("P2SC-HERE", "P2SC-GONE");
    let handle = r.plug(here);
    let gone_handle = r.plug(gone);
    r.hub.handle().detach(gone_handle).unwrap();
    let mut admin = r.client("Installer", Role::Admin);
    let mut other = r.client("Second admin", Role::Admin);
    let id = admin.add(volume_rule("Doors", &[here, gone, "P2SC-NEVER"], -33.0));

    let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
    let rows = ran["results"].as_array().unwrap();
    assert_eq!(rows.len(), 6, "{ran}");
    let row = |serial: &str, action: usize| {
        rows.iter()
            .find(|r| r["serial"] == serial && r["action"] == format!("{action}:param.set"))
            .cloned()
            .unwrap()
    };
    assert_eq!(row(here, 0)["ok"], true);
    assert_eq!(code(&row(gone, 0)), "offline", "{ran}");
    assert_eq!(code(&row(gone, 1)), "offline");
    assert_eq!(code(&row("P2SC-NEVER", 0)), "not_found");
    // Nothing waits for the offline device: plugged back, it gets nothing.
    r.plug(gone);
    assert_ne!(admin.get(gone, "vol.master"), -33.0, "never retried later");

    // Another session's guarded apply pending on the device: the action is `conflict`.
    let begin = other.ask(json!({"t": "apply.begin", "handle": handle, "timeout_ms": 60_000}));
    assert!(ok(&begin), "{begin}");
    let before = admin.get(here, "vol.master");
    let mut quiet = volume_rule("Quiet", &[here], -40.0);
    quiet["actions"] = json!([{"t": "param.set", "path": "vol.master", "value": -40.0}]);
    let quiet = admin.add(quiet);
    let ran = admin.ask(json!({"t": "schedule.run_now", "rule": quiet}));
    assert_eq!(code(&ran["results"][0]), "conflict", "{ran}");
    assert_eq!(ran["results"][0]["ok"], false);
    assert_eq!(admin.get(here, "vol.master"), before, "nothing sent");
    let record = loop {
        let rec = r.schedule_record(here);
        if rec.source.as_deref() == Some(quiet.as_str()) {
            break rec;
        }
    };
    assert!(record.text.contains("failed"), "{}", record.text);
    assert!(record.lines[0].contains("conflict"), "{:?}", record.lines);
    r.hub.stop();
}

#[test]
fn wp_p2sc_a_limit_refuses_an_action_and_nothing_is_sent() {
    let r = rig("limit");
    let serial = "P2SC-LIMIT";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let set = admin.ask(json!({"t": "limits.set", "handle": handle,
        "profile": {"masterVolumeMaxDb": -10.0}}));
    assert!(ok(&set), "{set}");
    let mut rule = volume_rule("Loud", &[serial], -5.0);
    rule["actions"] = json!([{"t": "param.set", "path": "vol.master", "value": -5.0},
                             {"t": "param.set", "path": "vol.master", "value": -15.0},
                             {"t": "system.scene.apply", "scene": "show"}]);
    let id = admin.add(rule);
    let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
    let rows = ran["results"].as_array().unwrap();
    assert_eq!(code(&rows[0]), "limit_exceeded", "{ran}");
    assert_eq!(rows[1]["ok"], true, "{ran}");
    // A scene is the audio link's, not a target's: one row; this hub has no such scene.
    assert_eq!(rows.len(), 3, "{ran}");
    assert_eq!(rows[2]["action"], "2:system.scene.apply");
    assert_eq!(code(&rows[2]), "not_found", "{ran}");
    assert_eq!(
        admin.get(serial, "vol.master"),
        -15.0,
        "the one inside the limit"
    );
    let list = admin.ask(json!({"t": "schedule.list"}));
    assert_eq!(list["rules"][0]["last"]["ok"], false);
    r.hub.stop();
}

#[test]
fn wp_p2sc_a_rule_runs_on_its_time_and_a_jump_forward_skips_and_records() {
    let r = rig("timer");
    let serial = "P2SC-TIMER";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut rule = volume_rule("Noon", &[serial], -18.0);
    rule["when"] = json!({"days": ["mon"], "time": "12:01"});
    let noon = admin.add(rule);
    let mut rule = volume_rule("Afternoon", &[serial], -30.0);
    rule["when"] = json!({"days": ["mon"], "time": "13:00"});
    let afternoon = admin.add(rule);

    // 12:01:10: the hub's timer runs it, with no one asking.
    r.advance(Duration::from_secs(70));
    let by = r.next_on_bus("the scheduled run", |m| match m {
        BusMessage::ScheduleRan {
            rule, by, results, ..
        } if *rule == noon => {
            assert!(results.iter().all(|x| x.ok), "{results:?}");
            Some(by.clone())
        }
        _ => None,
    });
    assert_eq!(by, None, "on its schedule, no admin");
    assert_eq!(admin.get(serial, "vol.master"), -18.0);
    let record = r.schedule_record(serial);
    assert_eq!(record.by.as_deref(), Some("Schedule: Noon"));

    // The clock is set two hours on: 13:00 was passed by the jump, so it is skipped
    // (recorded in the history and in `last`), not run.
    r.set_wall(MONDAY_NOON + 2 * 3600 + 70);
    let record = r.schedule_record(serial);
    assert!(record.text.contains("skipped"), "{}", record.text);
    assert_eq!(record.source.as_deref(), Some(afternoon.as_str()));
    assert_eq!(record.by.as_deref(), Some("Schedule: Afternoon"));
    let list = admin.ask(json!({"t": "schedule.list"}));
    let rules = list["rules"].as_array().unwrap();
    let a = rules.iter().find(|x| x["id"] == afternoon).unwrap();
    assert_eq!(
        a["last"],
        json!({"at": "2027-01-04T13:00:00Z", "ok": false, "skipped": true})
    );
    assert_eq!(a["next_run"], "2027-01-11T13:00:00Z");
    let n = rules.iter().find(|x| x["id"] == noon).unwrap();
    assert_eq!(n["last"]["ok"], true);
    assert_eq!(n["next_run"], "2027-01-11T12:01:00Z");
    assert_eq!(
        admin.get(serial, "vol.master"),
        -18.0,
        "the skipped one sent nothing"
    );

    // Back two hours: 12:01 comes round again, and does not run twice in a day.
    r.set_wall(MONDAY_NOON + 30);
    r.advance(Duration::from_secs(120));
    std::thread::sleep(Duration::from_millis(500));
    while let Some(m) = r.bus.try_recv() {
        assert!(
            !matches!(&*m, BusMessage::ScheduleRan { .. }),
            "ran again after a backward jump: {m:?}"
        );
    }
    assert_eq!(admin.get(serial, "vol.master"), -18.0);
    r.hub.stop();
}

#[test]
fn wp_p2sc_nothing_runs_while_the_clock_is_unset_and_the_list_says_so() {
    let r = rig("unset");
    let serial = "P2SC-UNSET";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let mut rule = volume_rule("Doors", &[serial], -20.0);
    rule["when"] = json!({"days": ["wed"], "time": "00:01"});
    let id = admin.add(rule);
    // A hub without a battery clock: 2020-01-01 00:00 (a Wednesday) after a reboot.
    r.set_wall(1_577_836_800);
    let list = admin.ask(json!({"t": "schedule.list"}));
    assert_eq!(list["clock_unset"], true);
    assert_eq!(list["rules"][0]["next_run"], Value::Null);
    let ran = admin.ask(json!({"t": "schedule.run_now", "rule": id}));
    assert_eq!(code(&ran), "conflict", "{ran}");
    // Its 00:01 passes: nothing runs.
    r.advance(Duration::from_secs(120));
    // Set by the network: schedules run again.
    r.set_wall(MONDAY_NOON + 60);
    let start = Instant::now();
    while admin.ask(json!({"t": "schedule.list"}))["clock_unset"] != false {
        assert!(start.elapsed() < WAIT, "the clock stayed unset");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    // The alarm is P2AL's: the schedule publishes none, and ran nothing meanwhile.
    while let Some(m) = r.bus.try_recv() {
        assert!(
            !matches!(
                &*m,
                BusMessage::Alarm { .. } | BusMessage::ScheduleRan { .. }
            ),
            "{m:?}"
        );
    }
    r.hub.stop();
}

#[test]
fn wp_p2sc_rules_and_their_last_run_survive_a_restart() {
    let dir = scratch("restart");
    let clock = Arc::new(TestClock::at(wall(MONDAY_NOON)));
    let serial = "P2SC-KEEP";
    {
        let r = rig_at(&dir, Arc::clone(&clock));
        r.plug(serial);
        let mut admin = r.client("Installer", Role::Admin);
        let id = admin.add(volume_rule("Doors", &[serial], -20.0));
        assert!(ok(&admin.ask(json!({"t": "schedule.run_now", "rule": id}))));
        r.hub.stop();
    }
    let file = dir.join("store").join("schedule.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let r = rig_at(&dir, Arc::clone(&clock));
    let mut viewer = r.client("Watcher", Role::Viewer);
    let list = viewer.ask(json!({"t": "schedule.list"}));
    assert_eq!(list["rules"][0]["name"], "Doors", "{list}");
    assert_eq!(list["rules"][0]["last"]["ok"], true);
    r.hub.stop();
    // A torn file: kept aside, the hub still starts and answers.
    std::fs::write(&file, "{\"rules\": [ {\"id\": 7").unwrap();
    let r = rig_at(&dir, clock);
    let mut viewer = r.client("Watcher", Role::Viewer);
    let list = viewer.ask(json!({"t": "schedule.list"}));
    assert!(list["rules"].as_array().unwrap().is_empty(), "{list}");
    let aside = std::fs::read_dir(dir.join("store"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("schedule.json.bad-")
        });
    assert!(aside, "the torn file was kept aside");
    r.hub.stop();
}
