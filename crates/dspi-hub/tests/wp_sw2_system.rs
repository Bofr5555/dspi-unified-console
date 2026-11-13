//! WP SW2: the hub's system model against the fake device's audio link model
//! (`FakeOptions::audio_link`, `AudioLinkSetup`): a sender, a fan-out that
//! re-emits 3 frames late, a repeater and four units behind them, all attached to
//! one hub. The alignment is computed from what they report and written to them;
//! scenes go through the sender or to each unit; a lost link raises an alarm; and
//! the model persists and is served as `system.*` over Link.
#![allow(clippy::arithmetic_side_effects)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::system::model::*;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_proto::ext::audio_link::{self as alink, ApplyOp, ApplyRequest, LinkRole};
use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::{AudioLinkSetup, LINK_SIM_LOCK, LINK_SIM_LOSE, LINK_SIM_REANCHOR};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, InRead, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(10);

/// Every link write a device was sent: (sub-command, payload).
type Writes = Arc<Mutex<Vec<(u16, Vec<u8>)>>>;

/// A fake that records its link writes.
struct Logged {
    inner: FakeDevice,
    writes: Writes,
}

impl Transport for Logged {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        let r = self.inner.control_out(o, v, d);
        if o == op::REQ_EXTENSION && v & 0xFF00 == 0x8000 && r.is_ok() {
            self.writes.lock().unwrap().push((v, d.to_vec()));
        }
        r
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
    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>, TransportError>> {
        self.inner.control_in_batch(reads)
    }
}

/// One fake on the link, attached to `hub`.
struct Fake {
    writes: Writes,
    remote: FakeRemote,
}

fn attach(hub: &Hub, serial: &str, setup: AudioLinkSetup) -> Fake {
    let dev = FakeDevice::demo_audio_link_as(setup).with_serial(serial);
    let remote = dev.remote();
    let writes = Writes::default();
    hub.attach(
        Box::new(Logged {
            inner: dev,
            writes: Arc::clone(&writes),
        }),
        0,
    )
    .unwrap();
    Fake { writes, remote }
}

fn receiver(hops: u8) -> AudioLinkSetup {
    AudioLinkSetup {
        hops,
        ..AudioLinkSetup::default()
    }
}

/// The installation: sender `tx`; fan-out `fan` (3 frames) on its output 1;
/// unit A on output 0; B behind the fan-out; repeater R (1 frame) behind the
/// fan-out; C behind R. All in one room.
struct Rig {
    tx: Fake,
    a: Fake,
    b: Fake,
    r: Fake,
    c: Fake,
}

fn rig(hub: &Hub) -> Rig {
    attach(
        hub,
        "SW2-FAN",
        AudioLinkSetup {
            role: LinkRole::FanOut,
            hop_delay_frames: 3,
            ..AudioLinkSetup::default()
        },
    );
    Rig {
        tx: attach(
            hub,
            "SW2-TX",
            AudioLinkSetup {
                role: LinkRole::Sender,
                ..AudioLinkSetup::default()
            },
        ),
        a: attach(hub, "SW2-A", receiver(0)),
        b: attach(hub, "SW2-B", receiver(1)),
        r: attach(
            hub,
            "SW2-R",
            AudioLinkSetup {
                role: LinkRole::Repeater,
                hops: 1,
                hop_delay_frames: 1,
                ..AudioLinkSetup::default()
            },
        ),
        c: attach(hub, "SW2-C", receiver(2)),
    }
}

fn unit(id: &str, upstream: &str) -> Unit {
    Unit {
        id: id.into(),
        name: format!("Unit {id}"),
        serial: format!("SW2-{id}"),
        room: Some("cinema".into()),
        upstream: upstream.into(),
        output: None,
        repeater: false,
        hop_delay_frames: None,
        slots: None,
        latency_us: 0,
        measured_latency_us: None,
        speakers: Vec::new(),
    }
}

fn model() -> SystemModel {
    let mut r = unit("R", "fan");
    r.repeater = true;
    let mut a = unit("A", "tx");
    a.speakers = vec![
        Speaker {
            output: 0,
            distance_m: 3.43,
        },
        Speaker {
            output: 1,
            distance_m: 2.0,
        },
    ];
    let mut b = unit("B", "fan");
    b.slots = Some(vec![4, 5, 255, 255, 255, 255, 255, 255]);
    SystemModel {
        rooms: vec![Room {
            id: "cinema".into(),
            name: "Cinema".into(),
            lip_sync_us: None,
        }],
        zones: vec![Zone {
            id: "downstairs".into(),
            name: "Downstairs".into(),
            rooms: vec!["cinema".into()],
            source: Some("film".into()),
        }],
        groups: vec![Group {
            id: "fronts".into(),
            name: "Fronts".into(),
            units: vec!["A".into(), "B".into()],
            note: String::new(),
        }],
        sources: vec![Source {
            id: "film".into(),
            name: "Film".into(),
            sender: Some("tx".into()),
            channels: (0..8).collect(),
        }],
        senders: vec![Sender {
            id: "tx".into(),
            name: "Sender".into(),
            serial: Some("SW2-TX".into()),
            sender_id: Some(0x42),
        }],
        fan_outs: vec![FanOut {
            id: "fan".into(),
            name: "Fan-out".into(),
            serial: Some("SW2-FAN".into()),
            upstream: "tx".into(),
            output: Some(1),
            hop_delay_frames: None,
        }],
        units: vec![a, b, r, unit("C", "R")],
        scenes: vec![
            Scene {
                id: "quiet".into(),
                name: "Quiet".into(),
                actions: vec![SceneAction {
                    target: "all".into(),
                    op: "master_volume".into(),
                    arg: 0,
                    value: -2000,
                }],
            },
            Scene {
                id: "on".into(),
                name: "On".into(),
                actions: vec![SceneAction {
                    target: "all".into(),
                    op: "unmute".into(),
                    arg: 0,
                    value: 0,
                }],
            },
            Scene {
                id: "fronts-preset".into(),
                name: "Fronts' preset".into(),
                actions: vec![
                    SceneAction {
                        target: "group:fronts".into(),
                        op: "preset_load".into(),
                        arg: 3,
                        value: 0,
                    },
                    SceneAction {
                        target: "zone:downstairs".into(),
                        op: "mute".into(),
                        arg: 0,
                        value: 0,
                    },
                ],
            },
        ],
        ..SystemModel::default()
    }
}

fn hub() -> Hub {
    Hub::start(HubConfig {
        initial_scan: false,
        ..HubConfig::in_process("SW2 hub")
    })
    .unwrap()
}

fn writes_of(f: &Fake, id: u16) -> Vec<Vec<u8>> {
    f.writes
        .lock()
        .unwrap()
        .iter()
        .filter(|(v, _)| *v == id)
        .map(|(_, d)| d.clone())
        .collect()
}

#[test]
fn wp_sw2_the_alignment_is_computed_from_the_units_and_applied() {
    let hub = hub();
    let rig = rig(&hub);
    assert_eq!(hub.system_set_model(model()).unwrap(), 1);

    let status = hub.system_status();
    assert_eq!(status.observed.len(), 6, "{:?}", status.observed.keys());
    assert_eq!(status.observed["SW2-FAN"].hop_delay_frames, Some(3));
    let a = &status.alignment;
    assert!(a.issues.is_empty(), "{:?}", a.issues);
    // C is behind 3 + 1 frames and needs 112: D = 116, picked by the engine.
    assert_eq!((a.d_frames, a.d_auto, a.rate_hz), (116, true, 48_000));
    let by: std::collections::BTreeMap<&str, _> =
        a.units.iter().map(|p| (p.unit.as_str(), p)).collect();
    assert_eq!((by["A"].d_local, by["A"].hops_offset), (116, 0));
    assert_eq!((by["B"].d_local, by["B"].hops_offset), (115, -2));
    assert_eq!((by["R"].d_local, by["R"].hops_offset), (115, -2));
    assert_eq!((by["C"].d_local, by["C"].hops_offset), (114, -2));
    for p in &a.units {
        // Frame F is heard everywhere at t_F + D.
        assert_eq!(i64::from(p.path_delay_frames) + p.effective, 116);
    }

    let report = hub.system_align(true);
    assert!(report.applied);
    // Output 0 is the furthest speaker: its 0 ms is what the unit has already.
    let failed: Vec<_> = report
        .writes
        .iter()
        .filter(|w| w.result != "ok" && !(w.what == "out_delay:0" && w.result == "unchanged"))
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
    // What each unit was sent: phase lock and its delays (2.8.5's layout).
    let phase = |f: &Fake| writes_of(f, alink::EXT_LINK_PHASE);
    assert_eq!(phase(&rig.a), vec![vec![1, 0, 116, 0, 0, 0, 0, 0]]);
    assert_eq!(phase(&rig.b), vec![vec![1, 0, 115, 0, 0xFE, 0xFF, 0, 0]]);
    assert_eq!(phase(&rig.r), vec![vec![1, 0, 115, 0, 0xFE, 0xFF, 0, 0]]);
    assert_eq!(phase(&rig.c), vec![vec![1, 0, 114, 0, 0xFE, 0xFF, 0, 0]]);
    assert_eq!(
        writes_of(&rig.b, alink::EXT_LINK_SLOTS),
        vec![vec![4, 5, 255, 255, 255, 255, 255, 255]]
    );
    assert!(
        report
            .writes
            .iter()
            .any(|w| w.element == "A" && w.what == "out_delay:1" && w.result == "ok")
    );

    // The units read back what was written: nothing to write again, so
    // nothing re-anchors.
    let again = hub.system_align(true);
    let changed: Vec<_> = again
        .writes
        .iter()
        .filter(|w| w.result != "unchanged")
        .collect();
    assert!(changed.is_empty(), "{changed:?}");
    assert_eq!(phase(&rig.a).len(), 1);
    let seen = hub.system_status().observed;
    assert_eq!(seen["SW2-C"].phase, Some((1, 114, -2, 0)));
    hub.stop();
}

#[test]
fn wp_sw2_a_scene_reaches_every_unit_at_one_frame() {
    let hub = hub();
    let rig = rig(&hub);
    hub.system_set_model(model()).unwrap();

    // One op for everyone: through the sender's superframe.
    let quiet = hub.system_apply_scene("quiet").unwrap();
    assert_eq!(quiet.plan.path, "superframe");
    assert_eq!(quiet.writes.len(), 1);
    assert_eq!(quiet.writes[0].result, "ok", "{:?}", quiet.writes);
    let sent = writes_of(&rig.tx, alink::EXT_LINK_CTRL_MSG);
    assert_eq!(sent.len(), 1);
    let req = ApplyRequest::decode(&sent[0]).unwrap();
    assert_eq!(
        (req.frame, req.op, req.value),
        (quiet.plan.frame, ApplyOp::MasterVolume, -2000)
    );
    assert_eq!(req.frame % 32, 0);
    assert!(writes_of(&rig.a, alink::EXT_APPLY_AT_FRAME).is_empty());

    // UNMUTE goes to each unit, past each unit's BULKFIX guard.
    let on = hub.system_apply_scene("on").unwrap();
    assert_eq!(on.plan.path, "per_unit");
    assert!(
        on.writes.iter().all(|w| w.result == "ok"),
        "{:?}",
        on.writes
    );
    for f in [&rig.a, &rig.b, &rig.r, &rig.c] {
        let w = writes_of(f, alink::EXT_APPLY_AT_FRAME);
        assert_eq!(w.len(), 1);
        let r = ApplyRequest::decode(&w[0]).unwrap();
        assert_eq!((r.frame, r.op), (on.plan.frame, ApplyOp::Unmute));
    }

    // Several ops, a group and a zone: per unit, every op at the same frame.
    let fronts = hub.system_apply_scene("fronts-preset").unwrap();
    assert_eq!(fronts.plan.path, "per_unit");
    assert_eq!(fronts.writes.len(), 2 + 4);
    assert!(
        fronts.writes.iter().all(|w| w.result == "ok"),
        "{:?}",
        fronts.writes
    );
    let a = writes_of(&rig.a, alink::EXT_APPLY_AT_FRAME);
    let ops: Vec<(u64, ApplyOp)> = a[1..]
        .iter()
        .map(|b| ApplyRequest::decode(b).map(|r| (r.frame, r.op)).unwrap())
        .collect();
    assert_eq!(
        ops,
        [
            (fronts.plan.frame, ApplyOp::PresetLoad),
            (fronts.plan.frame, ApplyOp::Mute)
        ]
    );
    assert!(hub.system_apply_scene("nope").is_err());
    hub.stop();
}

/// With the sender detached, a single-op scene falls back to the per-unit path.
#[test]
fn wp_sw2_without_the_sender_a_scene_goes_per_unit() {
    let hub = hub();
    let a = attach(&hub, "SW2-A", receiver(0));
    let mut m = model();
    m.units.retain(|u| u.id == "A");
    m.groups.clear();
    m.scenes.retain(|s| s.id == "quiet");
    hub.system_set_model(m).unwrap();
    let r = hub.system_apply_scene("quiet").unwrap();
    assert_eq!(r.plan.path, "per_unit");
    assert_eq!(r.writes[0].result, "ok");
    assert_eq!(writes_of(&a, alink::EXT_APPLY_AT_FRAME).len(), 1);
    // Alignment of a lone unit: D is its own minimum.
    assert_eq!(hub.system_status().alignment.d_frames, 112);
    hub.stop();
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "no {what} in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_sw2_a_lost_link_raises_an_alarm_and_a_lock_clears_it() {
    let hub = hub();
    let rig = rig(&hub);
    hub.system_set_model(model()).unwrap();
    // The first read learns which handle is which unit and that it sends 0xE1.
    assert!(hub.system_status().alarms.is_empty());

    assert!(rig.b.remote.link_sim(LINK_SIM_LOSE));
    let lost = || {
        hub.system_alarms()
            .iter()
            .any(|a| a.code == "link_lost" && a.subject == "B" && a.cleared_at.is_none())
    };
    wait_for("link_lost alarm from the event", lost);
    // A read agrees: B is in holdover.
    let s = hub.system_status();
    assert_eq!(s.observed["SW2-B"].state.as_deref(), Some("holdover"));
    let alarm = s
        .alarms
        .iter()
        .find(|a| a.code == "link_lost")
        .unwrap()
        .clone();
    let acked = hub.system_acknowledge(alarm.id).unwrap();
    assert_eq!(
        acked.state,
        dspi_hub::system::alarms::AlarmState::Acknowledged
    );

    assert!(rig.b.remote.link_sim(LINK_SIM_LOCK));
    wait_for("the alarm to clear", || !lost());

    // A re-anchor nobody asked for is an alarm; the hub's own (the alignment's
    // phase writes) is not: the over-Link test sees none after its writes.
    assert!(rig.a.remote.link_sim(LINK_SIM_REANCHOR));
    wait_for("a reanchor alarm", || {
        hub.system_alarms()
            .iter()
            .any(|a| a.code == "reanchor" && a.subject == "A")
    });

    // A unit that leaves the hub raises unit_offline at the next read.
    let handle = hub
        .devices()
        .into_iter()
        .find(|d| d.identity.serial == "SW2-C")
        .unwrap()
        .handle;
    hub.detach(handle).unwrap();
    let s = hub.system_status();
    assert!(
        s.alarms
            .iter()
            .any(|a| a.code == "unit_offline" && a.subject == "C")
    );
    assert!(s.alignment.issues.iter().any(|i| i.code == "offline"));
    hub.stop();
}

// ---- Over Link, with the store ----

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-sw2").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn networked(dir: PathBuf, access: AccessMode) -> Hub {
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        initial_scan: false,
        ..HubConfig::in_process("SW2 hub")
    })
    .unwrap()
}

type Ws = WebSocket<TcpStream>;

fn ws(hub: &Hub) -> Ws {
    let port = hub.network().unwrap().port();
    let url = format!("ws://127.0.0.1:{port}/dspi/v1");
    let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let (mut ws, _) = tungstenite::client(req, tcp).unwrap();
    ws.send(Message::text(
        r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"sw2","app":"test","version":"0"}}"#,
    ))
    .unwrap();
    next(&mut ws, &["hello"]);
    ws
}

fn next(ws: &mut Ws, t: &[&str]) -> Value {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no {t:?} in time");
        if let Message::Text(text) = ws.read().unwrap() {
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            if t.iter().any(|t| v["t"] == *t) {
                return v;
            }
        }
    }
}

fn request(ws: &mut Ws, v: Value) -> Value {
    ws.send(Message::text(v.to_string())).unwrap();
    next(ws, &["ok", "err"])
}

#[test]
fn wp_sw2_the_model_over_link_and_across_a_restart() {
    let dir = store_dir();
    let hub = networked(dir.clone(), AccessMode::Open);
    let _rig = rig(&hub);
    let mut c = ws(&hub);

    let empty = request(&mut c, json!({"t": "system.get", "id": 1}));
    assert_eq!(empty["revision"], 0, "{empty}");
    assert_eq!(empty["model"]["units"], json!([]));

    // A bad model is refused with the reason, and changes nothing.
    let mut bad = serde_json::to_value(model()).unwrap();
    bad["units"][0]["upstream"] = "attic".into();
    let r = request(&mut c, json!({"t": "system.set", "id": 2, "model": bad}));
    assert_eq!(
        (r["t"].as_str(), r["code"].as_str()),
        (Some("err"), Some("bad_request"))
    );
    assert!(r["msg"].as_str().unwrap().contains("attic"), "{r}");
    let r = request(
        &mut c,
        json!({"t": "system.set", "id": 3, "model": {"units": 7}}),
    );
    assert_eq!(r["code"], "bad_request");

    // The answer and the event every session gets, in either order.
    let set = json!({"t": "system.set", "id": 4, "model": serde_json::to_value(model()).unwrap()});
    c.send(Message::text(set.to_string())).unwrap();
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let v = next(&mut c, &["ok", "err", "system.changed"]);
        assert_eq!(v["revision"], 1, "{v}");
        seen.push(v["t"].as_str().unwrap().to_owned());
    }
    seen.sort();
    assert_eq!(seen, ["ok", "system.changed"]);

    let s = request(&mut c, json!({"t": "system.status", "id": 5}));
    assert_eq!(s["alignment"]["dFrames"], 116, "{s}");
    assert_eq!(s["observed"]["SW2-FAN"]["hopDelayFrames"], 3);

    let dry = request(&mut c, json!({"t": "system.align", "id": 6}));
    assert_eq!(dry["applied"], false);
    assert_eq!(dry["writes"], json!([]));
    let done = request(&mut c, json!({"t": "system.align", "id": 7, "apply": true}));
    assert!(
        done["writes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["result"] == "ok" || w["result"] == "unchanged"),
        "{done}"
    );

    let scene = request(
        &mut c,
        json!({"t": "system.scene.apply", "id": 8, "scene": "quiet"}),
    );
    assert_eq!(scene["plan"]["path"], "superframe", "{scene}");
    let none = request(
        &mut c,
        json!({"t": "system.scene.apply", "id": 9, "scene": "x"}),
    );
    assert_eq!(none["code"], "bad_request");
    let alarms = request(&mut c, json!({"t": "system.alarms", "id": 10}));
    assert_eq!(alarms["alarms"], json!([]));
    let ack = request(
        &mut c,
        json!({"t": "system.alarm.ack", "id": 11, "alarm": 99}),
    );
    assert_eq!(ack["code"], "bad_request");
    let unknown = request(&mut c, json!({"t": "system.nope", "id": 12}));
    assert_eq!(unknown["code"], "unknown_type");
    drop(c);
    hub.stop();

    // The model is in the store: a new hub on the same directory serves it.
    let text = fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(text.contains("\"system\""));
    let hub = networked(dir, AccessMode::Open);
    let mut c = ws(&hub);
    let back = request(&mut c, json!({"t": "system.get", "id": 1}));
    assert_eq!(
        serde_json::from_value::<SystemModel>(back["model"].clone()).unwrap(),
        model()
    );
    drop(c);
    hub.stop();
}

#[test]
fn wp_sw2_roles_reading_for_all_changing_for_admin() {
    let hub = networked(store_dir(), AccessMode::Pin);
    let net = hub.network().unwrap();
    let pair = |role: &str| {
        let mut c = ws(&hub);
        let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
        let r = request(
            &mut c,
            json!({"t": "auth.pair", "id": 1, "pin": pin, "name": role, "role": role}),
        );
        assert_eq!(r["role"], role, "{r}");
        c
    };
    let mut viewer = pair("viewer");
    assert_eq!(
        request(&mut viewer, json!({"t": "system.get", "id": 2}))["t"],
        "ok"
    );
    assert_eq!(
        request(&mut viewer, json!({"t": "system.alarms", "id": 3}))["t"],
        "ok"
    );
    for t in [
        "system.set",
        "system.align",
        "system.scene.apply",
        "system.alarm.ack",
    ] {
        let r = request(&mut viewer, json!({"t": t, "id": 4}));
        assert_eq!(r["code"], "denied", "{t}: {r}");
    }
    let mut control = pair("control");
    for t in ["system.set", "system.align"] {
        let r = request(&mut control, json!({"t": t, "id": 5}));
        assert_eq!(r["code"], "denied", "{t}: {r}");
    }
    // Control may apply a scene: this one is refused for not existing, not denied.
    let r = request(
        &mut control,
        json!({"t": "system.scene.apply", "id": 6, "scene": "x"}),
    );
    assert_eq!(r["code"], "bad_request");
    drop((viewer, control));
    hub.stop();
}
