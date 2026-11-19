//! WP ATLATE (review AT-D2, "Left" 6): the hub refuses `system.guard.begin`,
//! `confirm` and `cancel` to a Viewer and to a Control client **over the
//! wire**, as `system/guard.rs` intends (Admin and the guard's owner only).
//! The hub here pairs with a PIN (`AccessMode::Pin`), so the clients are the
//! real `DspiCore` over DSPi Link with the roles the hub's own machine gives
//! them; the other ATGUARD tests run with open access, where every session
//! is Admin.
//!
//! What is shown: a Viewer and a Control client are answered `denied` for
//! begin (with no guard pending, so not `busy`) and for confirm and cancel
//! of an Admin's pending guard (so not `conflict`); none of it ends, starts
//! or changes a guard (`system.guard.status`, which every role may read,
//! still shows the Admin's guard with its revision, and no outcome); the
//! Admin then cancels it as its owner. A Control client made Admin may begin
//! a guard of its own after that, so the refusal is the role's.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_measure::system::messages::guard as wire;
use dspi_transport::{Capture, FakeDevice, FakeOptions};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const SERIAL: &str = "ATLATE-1";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-atlate").join(format!(
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

/// A PIN hub with one device.
fn hub_in(dir: &std::path::Path) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process("ATLATE hub")
    })
    .unwrap();
    let device = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(SERIAL);
    hub.attach(Box::new(device), 0).unwrap();
    hub
}

/// A core paired on `hub` with the hub's PIN (Control), given `role` on the
/// hub's machine, bound to the device. Also its client id.
fn client(hub: &Hub, role: Role) -> (Arc<DspiCore>, u32) {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-atlate".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
    if role != Role::Control {
        assert!(net.set_role(cid, role));
    }
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    (core, cid)
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

/// The hub's refusal: its code and words.
fn refusal(r: dspi_core::Result<Value>) -> (String, String) {
    match r {
        Err(DspiError::Link { code, msg }) => (code, msg),
        other => panic!("not a Link refusal: {other:?}"),
    }
}

fn status(core: &DspiCore) -> wire::StatusReply {
    serde_json::from_value(ok(core, wire::STATUS, json!({}))).unwrap()
}

/// `role` is refused begin, confirm and cancel, each `denied` with the
/// hub's words.
fn assert_refused(core: &DspiCore, role: &str, guard: Option<u64>) {
    let (code, msg) = refusal(request(core, wire::BEGIN, json!({"timeout_ms": 30_000})));
    assert_eq!(code, "denied", "{role} begin: {msg}");
    assert!(msg.contains("admin"), "{role} begin: {msg}");
    // A guard id that is pending (an Admin's) and one that is not: the
    // role is judged before the guard.
    for g in [guard.unwrap_or(1), 9_999] {
        for t in [wire::CONFIRM, wire::CANCEL] {
            let (code, msg) = refusal(request(core, t, json!({"guard": g})));
            assert_eq!(code, "denied", "{role} {t} {g}: {msg}");
            assert!(msg.contains("admin"), "{role} {t} {g}: {msg}");
        }
    }
    // Without a guard id too (not `bad_request`: the role first).
    let (code, msg) = refusal(request(core, wire::CONFIRM, json!({})));
    assert_eq!(code, "denied", "{role} confirm without an id: {msg}");
}

#[test]
fn wp_atlate_viewer_and_control_are_refused_the_system_guard_over_the_wire() {
    let dir = scratch("roles");
    let hub = hub_in(&dir);
    let (admin, _) = client(&hub, Role::Admin);
    let (control, control_cid) = client(&hub, Role::Control);
    let (viewer, _) = client(&hub, Role::Viewer);
    // The hub judges every command by the session's role now (a role
    // given after the hello applies to the next command; the client's own
    // copy of it is the hello's).
    let roles: Vec<Role> = hub
        .network()
        .unwrap()
        .auth()
        .clients()
        .iter()
        .map(|c| c.role)
        .collect();
    assert_eq!(roles, [Role::Admin, Role::Control, Role::Viewer]);
    let caps = admin.hub_caps().caps;
    assert!(caps.iter().any(|c| c == wire::CAP), "{caps:?}");

    // No guard pending: begin is refused for the role, not as busy.
    assert_refused(&viewer, "viewer", None);
    assert_refused(&control, "control", None);
    let st = status(&viewer);
    assert!(st.active.is_none() && st.last.is_none(), "{st:?}");

    // An Admin's guard pending: neither may begin one (not `busy`), nor
    // confirm or cancel it (not `conflict`).
    let g: wire::BeginReply =
        serde_json::from_value(ok(&admin, wire::BEGIN, json!({"timeout_ms": 60_000}))).unwrap();
    assert_refused(&viewer, "viewer", Some(g.guard));
    assert_refused(&control, "control", Some(g.guard));

    // Nothing of it touched the guard: still the Admin's, pending, not
    // reverting, nothing ended; every role reads that.
    for (who, c) in [
        ("viewer", &viewer),
        ("control", &control),
        ("admin", &admin),
    ] {
        let st = status(c);
        let a = st
            .active
            .as_ref()
            .unwrap_or_else(|| panic!("{who}: {st:?}"));
        assert_eq!(a.guard, g.guard, "{who}");
        assert_eq!(a.revision_filed, g.revision, "{who}");
        assert!(!a.reverting, "{who}: {st:?}");
        assert!(st.last.is_none(), "{who}: {st:?}");
    }

    // Its owner ends it.
    let o: wire::Outcome =
        serde_json::from_value(ok(&admin, wire::CANCEL, json!({"guard": g.guard}))).unwrap();
    assert_eq!(o.guard, g.guard);
    assert_eq!(o.reason, "cancelled");
    let st = status(&viewer);
    assert!(st.active.is_none(), "{st:?}");

    // The refusal was the role's: the same client made Admin may begin and
    // end a guard of its own.
    assert!(hub.network().unwrap().set_role(control_cid, Role::Admin));
    let g2: wire::BeginReply =
        serde_json::from_value(ok(&control, wire::BEGIN, json!({"timeout_ms": 60_000}))).unwrap();
    assert!(g2.guard > g.guard);
    let o: wire::Outcome =
        serde_json::from_value(ok(&control, wire::CONFIRM, json!({"guard": g2.guard}))).unwrap();
    assert_eq!(o.result, "kept");
    // And the Viewer is refused still.
    assert_refused(&viewer, "viewer", None);
}
