//! WP ATGUARD: the hub's guard of a system apply (`system.guard.*`,
//! `docs/integrator/hub-api.md` section 23.4) on the real in-process hub,
//! with two ATFAKE units on one line time base and their sender, reached
//! over real DSPi Link sessions (two consoles). The "apply" is what
//! `measure_system_apply` writes: measured arrivals into the model
//! (`system.set` at the revision read) and `system.align`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_measure::system::messages::guard as wire;
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::{AlinkSetup, FakeGeneration, FakeTimeBase, FakeTimeBaseUnit};
use serde_json::{Value, json};

const SERIALS: [&str; 2] = ["ATG-A", "ATG-B"];

fn store_dir() -> PathBuf {
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-atguard").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn connect(hub: &Hub, serial: &str) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-atguard".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let d = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == serial)
        .unwrap();
    core.link_select_device(d.handle, d.serial).unwrap();
    core
}

fn request(core: &DspiCore, t: &str, body: Value) -> dspi_core::Result<Value> {
    let Value::Object(m) = body else {
        panic!("an object")
    };
    core.link_hub_request(t, m)
}

fn ok(core: &DspiCore, t: &str, body: Value) -> Value {
    request(core, t, body).unwrap_or_else(|e| panic!("{t}: {e}"))
}

fn code(r: dspi_core::Result<Value>) -> String {
    match r {
        Err(DspiError::Link { code, .. }) => code,
        other => panic!("not a Link refusal: {other:?}"),
    }
}

fn out_delay_ms(core: &DspiCore, serial: &str) -> f64 {
    let v = ok(
        core,
        "param.get_many",
        json!({"serial": serial, "items": [{"path": "out.delay", "idx": [0]}]}),
    );
    v["values"][0]["value"].as_f64().unwrap()
}

struct Rig {
    hub: Hub,
    a: Arc<DspiCore>,
    b: Arc<DspiCore>,
    before: [f64; 2],
}

fn rig() -> Rig {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("wp-atguard hub")
    })
    .unwrap();
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));
    let tb = FakeTimeBase::line(0x31);
    for serial in SERIALS {
        let d = FakeDevice::demo_on_time_base_as(
            &tb,
            FakeTimeBaseUnit::Line(AlinkSetup::default()),
            FakeGeneration::Bofr116 { page_guard: true },
        )
        .with_serial(serial);
        hub.attach(Box::new(d), 0).unwrap();
    }
    hub.attach(
        Box::new(
            FakeDevice::demo_alink_as(AlinkSetup {
                role: AlinkRole::Sender,
                ..AlinkSetup::default()
            })
            .with_serial("ATG-TX"),
        ),
        0,
    )
    .unwrap();
    let model = json!({
        "rooms": [{"id": "main", "name": "Main"}],
        "senders": [{"id": "tx", "serial": "ATG-TX"}],
        "units": [
            {"id": "A", "serial": SERIALS[0], "room": "main", "upstream": "tx",
             "speakers": [{"output": 0, "distanceM": 2.0}]},
            {"id": "B", "serial": SERIALS[1], "room": "main", "upstream": "tx",
             "speakers": [{"output": 0, "distanceM": 2.0}]}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    hub.handle().system_align(true);
    let a = connect(&hub, SERIALS[0]);
    let b = connect(&hub, SERIALS[1]);
    assert_eq!(a.link_hub_role(), "admin");
    let before = [out_delay_ms(&a, SERIALS[0]), out_delay_ms(&a, SERIALS[1])];
    Rig { hub, a, b, before }
}

/// What `measure_system_apply` does inside the guard: the measured
/// arrivals into the model at the revision read (A heard 2 ms before B),
/// then the alignment. The revision it leaves.
fn apply(core: &DspiCore) -> u64 {
    let got = ok(core, "system.get", json!({}));
    let mut model = got["model"].clone();
    for (i, us) in [(0usize, -2_000.0), (1, 0.0)] {
        model["units"][i]["speakers"][0]["measuredArrivalUs"] = json!(us);
    }
    let saved = ok(
        core,
        "system.set",
        json!({"model": model, "revision": got["revision"]}),
    );
    ok(core, "system.align", json!({"apply": true}));
    saved["revision"].as_u64().unwrap()
}

fn begin(core: &DspiCore, timeout_ms: u32) -> wire::BeginReply {
    serde_json::from_value(ok(core, wire::BEGIN, json!({"timeout_ms": timeout_ms}))).unwrap()
}

fn status(core: &DspiCore) -> wire::StatusReply {
    serde_json::from_value(ok(core, wire::STATUS, json!({}))).unwrap()
}

fn delays(r: &Rig, core: &DspiCore) -> [f64; 2] {
    let _ = r;
    [
        out_delay_ms(core, SERIALS[0]),
        out_delay_ms(core, SERIALS[1]),
    ]
}

fn assert_applied(r: &Rig, now: [f64; 2]) {
    assert!((now[0] - r.before[0] - 2.0).abs() < 0.006, "{now:?}");
    assert!((now[1] - r.before[1]).abs() < 0.0015, "{now:?}");
}

fn assert_as_filed(r: &Rig, now: [f64; 2]) {
    assert!(
        (now[0] - r.before[0]).abs() < 0.0015 && (now[1] - r.before[1]).abs() < 0.0015,
        "{now:?} where {:?} was filed",
        r.before
    );
    let (model, _) = r.hub.handle().system_model();
    assert!(
        model
            .units
            .iter()
            .all(|u| u.speakers[0].measured_arrival_us.is_none()),
        "the filed model is back"
    );
}

/// Wait for the guard's outcome (the hub's thread reverts it).
fn wait_last(core: &DspiCore, guard: u64, within: Duration) -> wire::Outcome {
    let start = Instant::now();
    loop {
        let st = status(core);
        if let Some(o) = st.last.as_ref().filter(|o| o.guard == guard)
            && st.active.is_none()
        {
            return o.clone();
        }
        assert!(
            start.elapsed() < within,
            "guard {guard} did not end: {st:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// An Admin applies inside the guard (the cap in `hello` is proved by the
/// core's own apply in `review_atc_core_on_real_hub.rs`); the console
/// drops (its Link connection closes, no word): the hub reverts by itself,
/// the filed model at the revision the apply left and every delay back.
#[test]
fn wp_atguard_applied_then_the_console_drops_reverted() {
    let r = rig();
    let g = begin(&r.a, 300_000);
    assert_eq!(g.timeout_ms, 300_000);
    let left = apply(&r.a);
    assert!(left > g.revision);
    assert_applied(&r, delays(&r, &r.b));
    let st = status(&r.b);
    let active = st.active.unwrap();
    assert_eq!((active.guard, active.revision_filed), (g.guard, g.revision));
    assert_eq!(active.revision_left, left, "the owner's set is noted");
    assert!(active.remaining_ms > 290_000 && !active.reverting);
    // Gone.
    r.a.link_close();
    let o = wait_last(&r.b, g.guard, Duration::from_secs(10));
    assert_eq!(o.reason, "session_closed");
    assert_eq!(o.result, "reverted", "{o:?}");
    assert!(o.write_failures.is_empty(), "{o:?}");
    assert!(o.revision > left);
    assert_as_filed(&r, delays(&r, &r.b));
}

/// Keep before the deadline keeps it: confirmed, and the deadline passing
/// afterwards changes nothing.
#[test]
fn wp_atguard_keep_before_the_deadline_keeps_it() {
    let r = rig();
    let g = begin(&r.a, 1_500);
    assert_eq!(g.timeout_ms, 1_500);
    apply(&r.a);
    let o: wire::Outcome =
        serde_json::from_value(ok(&r.a, wire::CONFIRM, json!({"guard": g.guard}))).unwrap();
    assert_eq!(
        (o.reason.as_str(), o.result.as_str()),
        ("confirmed", "kept")
    );
    std::thread::sleep(Duration::from_millis(2_500));
    let st = status(&r.a);
    assert!(st.active.is_none());
    assert_eq!(st.last.unwrap().result, "kept");
    assert_applied(&r, delays(&r, &r.a));
    // A second confirm finds nothing pending.
    assert_eq!(
        code(request(&r.a, wire::CONFIRM, json!({"guard": g.guard}))),
        "conflict"
    );
}

/// No Keep: the hub reverts at the deadline, while the console is still
/// connected; a Keep after that is refused `conflict` (the client then
/// reads `status`).
#[test]
fn wp_atguard_no_keep_reverts_at_the_deadline() {
    let r = rig();
    let g = begin(&r.a, 1_000);
    apply(&r.a);
    assert_applied(&r, delays(&r, &r.a));
    let o = wait_last(&r.a, g.guard, Duration::from_secs(10));
    assert_eq!(
        (o.reason.as_str(), o.result.as_str()),
        ("timeout", "reverted"),
        "{o:?}"
    );
    assert_as_filed(&r, delays(&r, &r.a));
    assert_eq!(
        code(request(&r.a, wire::CONFIRM, json!({"guard": g.guard}))),
        "conflict"
    );
}

/// Revert ends it: `cancel` reverts now and answers how; nothing is
/// pending afterwards and the delays are as filed.
#[test]
fn wp_atguard_revert_ends_it() {
    let r = rig();
    let g = begin(&r.a, 300_000);
    apply(&r.a);
    let o: wire::Outcome =
        serde_json::from_value(ok(&r.a, wire::CANCEL, json!({"guard": g.guard}))).unwrap();
    assert_eq!(
        (o.reason.as_str(), o.result.as_str()),
        ("cancelled", "reverted"),
        "{o:?}"
    );
    assert!(o.writes_ok > 0, "{o:?}");
    assert!(status(&r.a).active.is_none());
    assert_as_filed(&r, delays(&r, &r.a));
    // A new guard may begin.
    let g2 = begin(&r.a, 300_000);
    assert!(g2.guard > g.guard);
    ok(&r.a, wire::CONFIRM, json!({"guard": g2.guard}));
}

/// While a guard is pending: another session's `system.set` is refused
/// `conflict` (and its `confirm`/`cancel` `denied`), a second `begin` is
/// `busy`; the owner's own `system.set` goes through. After the guard
/// ends, the other session may change the model again.
#[test]
fn wp_atguard_another_sessions_set_is_refused_while_pending() {
    let r = rig();
    let g = begin(&r.a, 300_000);
    apply(&r.a);
    let got = ok(&r.b, "system.get", json!({}));
    let set = || {
        request(
            &r.b,
            "system.set",
            json!({"model": got["model"], "revision": got["revision"]}),
        )
    };
    assert_eq!(code(set()), "conflict");
    assert_eq!(code(request(&r.b, wire::BEGIN, json!({}))), "busy");
    assert_eq!(
        code(request(&r.b, wire::CONFIRM, json!({"guard": g.guard}))),
        "denied"
    );
    assert_eq!(
        code(request(&r.b, wire::CANCEL, json!({"guard": g.guard}))),
        "denied"
    );
    assert_eq!(status(&r.b).active.unwrap().guard, g.guard, "still pending");
    // The owner may set again (its own edit inside the guard).
    let left = apply(&r.a);
    assert_eq!(status(&r.a).active.unwrap().revision_left, left);
    ok(&r.a, wire::CONFIRM, json!({"guard": g.guard}));
    let got = ok(&r.b, "system.get", json!({}));
    ok(
        &r.b,
        "system.set",
        json!({"model": got["model"], "revision": got["revision"]}),
    );
}

/// A model changed meanwhile by the hub's own API is not overwritten: the
/// revert is `not_written` and says so.
#[test]
fn wp_atguard_a_model_changed_meanwhile_is_not_overwritten() {
    let r = rig();
    let g = begin(&r.a, 300_000);
    apply(&r.a);
    let (model, _) = r.hub.handle().system_model();
    r.hub.handle().system_set_model(model).unwrap();
    let o: wire::Outcome =
        serde_json::from_value(ok(&r.a, wire::CANCEL, json!({"guard": g.guard}))).unwrap();
    assert_eq!(o.result, "not_written", "{o:?}");
    assert!(o.message.contains("changed"), "{o:?}");
    assert_applied(&r, delays(&r, &r.a));
}
