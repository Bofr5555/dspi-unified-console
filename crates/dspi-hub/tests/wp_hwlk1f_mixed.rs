//! WP HWLK1F: a unit whose firmware has no audio link (a K340 on bofr-main
//! 470a127: `EXT_LIST` names no 0x8000 block and the block stalls) in an
//! installation of the fake devices' audio link, on a real hub (hw-lk1's
//! findings F1-F5):
//!
//! - the hub says `alink: "none"` for it and the issue `no_alink`; every client
//!   (the core's view: Qt, Android) shows "No audio link (firmware)", never
//!   "Unknown", never a role from the model, and leaves it out of "N of M locked";
//! - a scene applies its ops at once, the others at the frame, and the outcome
//!   says so; "on time on every unit" only when it is true (an absent unit is
//!   named); a scene is refused only when no unit can apply it;
//! - aligning says what was written and what was skipped, and why; nothing
//!   written is said as such;
//! - `system.set` keeps a model with such a unit, with a warning;
//! - a PIN pairing gives at most control; admin is the role set on the hub's machine (PAIRADM).
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_proto::ext::alink::AlinkRole;
use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::AlinkSetup;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, InRead, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(10);

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hwlk1f").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// Every OUT the unit without the alink was sent: (opcode, wValue, payload).
type Outs = Arc<Mutex<Vec<(u8, u16, Vec<u8>)>>>;

/// The demo device (no audio link), recording its OUTs and its preset loads.
struct Plain {
    inner: FakeDevice,
    outs: Outs,
}

impl Transport for Plain {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_PRESET_LOAD {
            self.outs.lock().unwrap().push((o, v, Vec::new()));
        }
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_PRESET_LOAD {
            self.outs.lock().unwrap().push((o, v, Vec::new()));
        }
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        let r = self.inner.control_out(o, v, d);
        if r.is_ok() {
            self.outs.lock().unwrap().push((o, v, d.to_vec()));
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

/// The installation: sender, fan-out, units A, B (behind the fan-out), the
/// K340-like unit K without the alink, and unit D, which is never attached.
fn model(with_d: bool) -> Value {
    let mut units = vec![
        json!({"id": "A", "name": "Main left", "serial": "HW-A", "room": "main", "upstream": "tx", "output": 0}),
        json!({"id": "B", "name": "Main right", "serial": "HW-B", "room": "main", "upstream": "fan"}),
        json!({"id": "K", "name": "K340", "serial": "HW-K", "room": "main", "upstream": "tx",
               "speakers": [{"output": 0, "distanceM": 2.0}, {"output": 1, "distanceM": 2.5}]}),
    ];
    if with_d {
        units.push(
            json!({"id": "D", "name": "Bar", "serial": "HW-D", "room": "main", "upstream": "tx"}),
        );
    }
    json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "HW-FAN", "upstream": "tx", "output": 1}],
        "units": units,
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]},
            {"id": "k1", "name": "K out 1 delay", "actions": [{"target": "unit:K", "op": "out_delay", "arg": 0, "value": 48}]},
            {"id": "kmix", "name": "K mixed", "actions": [
                {"target": "unit:K", "op": "mute"},
                {"target": "unit:K", "op": "preset_load", "arg": 2},
                {"target": "unit:A", "op": "mute"}
            ]},
            {"id": "kclick", "name": "K click", "actions": [{"target": "unit:K", "op": "click", "arg": 0}]}
        ]
    })
}

/// A networked hub with the installation attached (D never is) and its model.
fn hub(access: AccessMode, with_d: bool) -> (Hub, Outs) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("HWLK1F hub")
    })
    .unwrap();
    let at = |role, hops, hop_delay_frames| AlinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AlinkSetup::default()
    };
    for (serial, setup) in [
        ("HW-TX", at(AlinkRole::Sender, 0, 0)),
        ("HW-FAN", at(AlinkRole::FanOut, 0, 3)),
        ("HW-A", at(AlinkRole::Receiver, 0, 0)),
        ("HW-B", at(AlinkRole::Receiver, 1, 0)),
    ] {
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    let outs = Outs::default();
    hub.attach(
        Box::new(Plain {
            inner: FakeDevice::demo().with_serial("HW-K"),
            outs: Arc::clone(&outs),
        }),
        0,
    )
    .unwrap();
    hub.handle()
        .system_set_model(serde_json::from_value(model(with_d)).unwrap())
        .unwrap();
    (hub, outs)
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-hwlk1f".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core
}

fn fresh_view(core: &DspiCore) -> dspi_core::link::AlinkSystemView {
    let start = Instant::now();
    loop {
        let v = core.alink_system_view().unwrap();
        if !v.stale {
            return v;
        }
        assert!(start.elapsed() < WAIT, "always stale");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn f32_at(b: &[u8]) -> f32 {
    f32::from_le_bytes(b[..4].try_into().unwrap())
}

/// F1: the hub tells a firmware without the alink from an unread one, and
/// every client says so: not Unknown, not a Receiver, not among the locked.
#[test]
fn wp_hwlk1f_a_unit_without_the_audio_link_is_shown_as_such() {
    let (hub, _) = hub(AccessMode::Open, false);
    let status = hub.handle().system_status();
    let k = &status.observed["HW-K"];
    let wire = serde_json::to_value(k).unwrap();
    assert_eq!(wire["alink"], "none", "{wire}");
    assert!(k.online && k.role.is_none() && k.state.is_none());
    assert_eq!(
        serde_json::to_value(&status.observed["HW-A"]).unwrap()["alink"],
        "present"
    );
    let codes: Vec<(&str, &str)> = status
        .alignment
        .issues
        .iter()
        .map(|i| (i.subject.as_deref().unwrap_or(""), i.code.as_str()))
        .collect();
    assert!(codes.contains(&("K", "no_alink")), "{codes:?}");
    assert!(!codes.contains(&("K", "limits_unknown")), "{codes:?}");
    assert!(
        !status.alarms.iter().any(|a| a.subject == "K"),
        "no alarm: {:?}",
        status.alarms
    );

    let core = connect(&hub);
    let v = fresh_view(&core);
    let row = v.units.iter().find(|u| u.id == "K").unwrap();
    assert_eq!(
        (
            row.state.as_str(),
            row.state_word.as_str(),
            row.role.as_str()
        ),
        ("no_alink", "No audio link (firmware)", "No audio link")
    );
    assert!(row.connected && !row.pending && row.alink_delay_frames.is_none());
    assert_eq!(
        v.unit_count, 2,
        "K is not counted among the units that lock"
    );
    assert_eq!(v.locked, 2);
    assert_eq!(
        v.summary,
        "Audio link: 2 of 2 locked, 1 without the audio link (firmware), audio link delay to write on 2"
    );
    assert_eq!(v.health, "warning");
    assert!(
        v.issues.iter().any(|i| i.contains("no audio link")),
        "{:?}",
        v.issues
    );
    hub.stop();
}

/// F2/F4: a scene for every unit goes through the sender for the units with
/// the alink and at once to K, and says so; one for K alone is applied at once,
/// not refused; one K cannot do (a click) is refused, nothing written.
#[test]
fn wp_hwlk1f_scenes_apply_at_once_on_a_unit_without_the_link() {
    let (hub, outs) = hub(AccessMode::Open, false);
    let h = hub.handle();
    h.system_status();

    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    assert_eq!(r.plan.path, "superframe", "{:?}", r.plan);
    assert_eq!(r.plan.direct.len(), 1);
    let k: Vec<_> = r.writes.iter().filter(|w| w.element == "K").collect();
    assert_eq!(k.len(), 1, "{:?}", r.writes);
    assert_eq!(k[0].result, "applied_now");
    assert_eq!(k[0].reason.as_deref(), Some("no audio link (firmware)"));
    assert!(!r.on_time, "K took it at once, not at the frame");
    {
        let outs = outs.lock().unwrap();
        let vol = outs
            .iter()
            .find(|(o, _, _)| *o == op::REQ_SET_MASTER_VOLUME)
            .expect("the master volume written to K");
        assert!((f32_at(&vol.2) + 12.0).abs() < 1e-4);
    }
    let core = connect(&hub);
    fresh_view(&core);
    let rc = core.alink_system_recall("evening".into()).unwrap();
    assert!(rc.taken && !rc.on_time);
    assert!(
        rc.message.starts_with(
            "Evening recalled: applied now on K340 (no audio link); at audio link frame "
        ) && rc.message.ends_with(" through the sender on the others."),
        "{}",
        rc.message
    );
    let kw = rc.writes.iter().find(|w| w.unit == "K340").unwrap();
    assert_eq!(
        (kw.what.as_str(), kw.result_word.as_str()),
        ("Master volume", "Applied now")
    );

    // K alone: at once, never "no unit reports an alink frame".
    let r = h.system_apply_scene("k1", Role::Admin).unwrap();
    assert_eq!((r.plan.path.as_str(), r.writes.len()), ("direct", 1));
    assert_eq!(r.writes[0].result, "applied_now");
    {
        let outs = outs.lock().unwrap();
        let d = outs
            .iter()
            .rev()
            .find(|(o, _, _)| *o == op::REQ_SET_OUTPUT_DELAY)
            .expect("out.delay written to K");
        // 48 samples at 48 kHz, as the firmware's apply rounds: 1.0104 ms.
        assert_eq!(d.1, 0);
        assert!(
            (f32_at(&d.2) - 1.010_416_7).abs() < 1e-4,
            "{}",
            f32_at(&d.2)
        );
    }
    // Mixed: K's mute and preset now, A's mute at the frame.
    let r = h.system_apply_scene("kmix", Role::Admin).unwrap();
    assert_eq!(r.plan.path, "per_unit");
    assert_eq!((r.plan.ops.len(), r.plan.direct.len()), (1, 2));
    let results: Vec<(&str, &str)> = r
        .writes
        .iter()
        .map(|w| (w.element.as_str(), w.result.as_str()))
        .collect();
    assert!(results.contains(&("A", "queued")), "{results:?}");
    assert_eq!(
        results
            .iter()
            .filter(|(e, x)| *e == "K" && *x == "applied_now")
            .count(),
        2
    );
    {
        let outs = outs.lock().unwrap();
        assert!(
            outs.iter()
                .any(|(o, _, p)| *o == op::REQ_SET_USER_MUTE && p == &[1])
        );
        assert!(
            outs.iter()
                .any(|(o, v, _)| *o == op::REQ_PRESET_LOAD && *v == 2)
        );
    }
    // A click has no direct command: not applied; nothing else to apply.
    let r = h.system_apply_scene("kclick", Role::Admin).unwrap();
    assert_eq!(r.writes[0].result, "unsupported");
    let rc = core.alink_system_recall("kclick".into()).unwrap();
    assert!(!rc.taken, "{}", rc.message);
    assert_eq!(
        rc.message,
        "K click was refused by every unit; nothing changed."
    );
    hub.stop();
}

/// F2: through the sender, a unit of the scene that is not attached is named
/// and the scene is not "on time on every unit".
#[test]
fn wp_hwlk1f_an_absent_unit_is_never_on_time() {
    let (hub, _) = hub(AccessMode::Open, true);
    let h = hub.handle();
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    assert_eq!(r.plan.path, "superframe");
    let d = r.writes.iter().find(|w| w.element == "D").expect("D named");
    assert_eq!(
        (d.result.as_str(), d.reason.as_deref()),
        ("no_device", Some("not connected"))
    );
    assert!(!r.on_time);
    let core = connect(&hub);
    fresh_view(&core);
    let rc = core.alink_system_recall("evening".into()).unwrap();
    assert!(rc.message.contains("; Bar not connected"), "{}", rc.message);
    assert!(!rc.message.contains("on time on every unit"));
    hub.stop();
}

/// F3: align says what it wrote and what it skipped, and why; when it wrote
/// nothing, it says that.
#[test]
fn wp_hwlk1f_align_says_what_was_written_and_skipped() {
    let (hub, outs) = hub(AccessMode::Open, true);
    let core = connect(&hub);
    fresh_view(&core);
    let a = core.alink_system_align().unwrap();
    assert!(a.applied);
    assert!(
        a.message
            .starts_with("The audio link delays were written on Main left, Main right")
            && a.message
                .contains("skipped on K340 (no audio link (firmware)), Bar (not connected)"),
        "{}",
        a.message
    );
    assert!(
        !outs
            .lock()
            .unwrap()
            .iter()
            .any(|(o, _, _)| *o == op::REQ_SET_OUTPUT_DELAY),
        "nothing written to K"
    );
    let report = hub.handle().system_align(true);
    let k = report.writes.iter().find(|w| w.element == "K").unwrap();
    assert_eq!(
        (k.result.as_str(), k.reason.as_deref()),
        ("skipped", Some("no audio link (firmware)"))
    );

    // Only K in the model: nothing written, said so.
    let only_k = json!({
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "units": [{"id": "K", "name": "K340", "serial": "HW-K", "upstream": "tx"}]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(only_k).unwrap())
        .unwrap();
    fresh_view(&core);
    let a = core.alink_system_align().unwrap();
    assert_eq!(
        a.message,
        "Nothing was written: skipped on K340 (no audio link (firmware))."
    );
    hub.stop();
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
        r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"hwlk1f","app":"test","version":"0"}}"#,
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

/// Decision 1 at the wire: `system.set` keeps a model naming a unit without
/// the alink, and warns.
#[test]
fn wp_hwlk1f_saving_a_unit_without_the_link_warns() {
    let (hub, _) = hub(AccessMode::Open, false);
    let mut c = ws(&hub);
    request(&mut c, json!({"t": "auth.pair", "id": 1, "pin": "x"}));
    let r = request(
        &mut c,
        json!({"t": "system.set", "id": 2, "model": model(false)}),
    );
    assert_eq!(r["t"], "ok", "{r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{r}");
    let w = warnings[0].as_str().unwrap();
    assert!(
        w.starts_with("K340 (HW-K): its firmware has no audio link, so it cannot be a receiver"),
        "{w}"
    );
    assert_eq!(hub.handle().system_model().0.units.len(), 3, "kept");
    hub.stop();
}

/// Decision 4 at the wire: a PIN gives at most control, whatever is asked.
/// -- WP PAIRADM -- Admin only as the role set on the hub's machine
/// (`uconsole hub role <cid> admin`), which applies live: no pairing gives it.
#[test]
fn wp_hwlk1f_a_pin_asking_for_admin_gets_control() {
    let (hub, _) = hub(AccessMode::Pin, false);
    let net = hub.network().unwrap();
    let mut c = ws(&hub);
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let r = request(
        &mut c,
        json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "asks", "role": "admin"}),
    );
    assert_eq!(
        (r["t"].as_str(), r["role"].as_str()),
        (Some("ok"), Some("control")),
        "{r}"
    );
    assert_eq!(
        request(&mut c, json!({"t": "system.align", "id": 2}))["code"],
        "denied"
    );
    assert_eq!(net.auth().clients()[0].role, Role::Control);

    // `uconsole hub role <cid> admin`: the same session, its next message
    let cid = net.auth().clients()[0].cid;
    assert!(net.set_role(cid, Role::Admin));
    assert_eq!(
        request(&mut c, json!({"t": "system.align", "id": 3}))["t"],
        "ok"
    );
    // A second pairing that asks for admin is control again.
    let mut c = ws(&hub);
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    let r = request(
        &mut c,
        json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "installer", "role": "admin"}),
    );
    assert_eq!(r["role"], "control", "{r}");
    hub.stop();
}
