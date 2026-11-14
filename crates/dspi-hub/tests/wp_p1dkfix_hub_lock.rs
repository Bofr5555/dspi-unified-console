//! WP P1DKFIX: on a hub-managed device the core defers to the hub's installer
//! lock (one lock, one prompt), and the hub's named refusals reach the core as
//! its own errors (F5). The client is the real one (`DspiCore` over DSPi
//! Link), the server the real one, a Control client paired with a PIN and an
//! Admin beside it.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const HUB_PIN: &str = "2468";

fn hub() -> Hub {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p1dkfix-link").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process("P1DKFIX hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial("DK-HUB")), 0)
        .unwrap();
    hub
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A core paired on `hub` (as an Admin when `admin`), bound to the device.
fn client(hub: &Hub, admin: bool) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let port = net.port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-p1dkfix".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    if admin {
        // The newest client is this one: made an Admin on the hub's machine.
        let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
        assert!(net.set_role(cid, Role::Admin));
    }
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

fn ask(core: &DspiCore, t: &str, mut fields: Value) -> Value {
    fields["handle"] = json!(core.link_devices().remove(0).handle);
    let Value::Object(f) = fields else {
        unreachable!()
    };
    core.link_hub_request(t, f).unwrap()
}

fn code_of(e: &DspiError) -> &str {
    match e {
        DspiError::InstallerLock { code, .. } => code,
        _ => "",
    }
}

#[test]
fn wp_p1dkfix_a_hub_managed_device_shows_one_lock_the_hubs() {
    let hub = hub();
    let control = client(&hub, false);
    let admin = client(&hub, true);
    assert_eq!(control.link_hub_role(), "control");

    // No hub PIN: the console's own lock is the one (and none is set).
    let st = control.installer_lock_refresh().unwrap();
    assert_eq!((st.held_by.as_str(), st.pin_set), ("", false), "{st:?}");
    // The console sets its own PIN for the device, and locks.
    control.installer_pin_set("1357".into()).unwrap();
    control.installer_lock_cover(vec!["front".into()]).unwrap();
    let st = control.installer_lock().unwrap();
    assert_eq!(
        (st.held_by.as_str(), st.locked),
        ("console", true),
        "{st:?}"
    );

    // The hub's installer locks the device on the hub and pins a mode.
    let profile = json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "lockedPages": ["eq"]});
    assert_eq!(
        ask(&admin, "limits.set", json!({"profile": profile}))["t"],
        "ok"
    );
    assert_eq!(
        ask(&admin, "limits.pin", json!({"pin": HUB_PIN}))["t"],
        "ok"
    );
    let mut f = serde_json::Map::new();
    f.insert("role".into(), json!("control"));
    f.insert("mode".into(), json!("simple"));
    admin.link_hub_request("mode.pin", f).unwrap();

    // One lock: the hub's. The console's own PIN asks nothing now.
    let st = control.installer_lock_refresh().unwrap();
    assert_eq!(st.held_by, "hub", "{st:?}");
    assert!(
        st.pin_set && st.locked && !st.unlocked && !st.installer_open,
        "{st:?}"
    );
    assert_eq!(st.covers, ["eq"]);
    assert_eq!(st.scope_notice_key, "installer_lock.hub");
    assert_eq!(st.hub_pinned_mode, "simple");
    assert_eq!(control.mode_pinned(), "simple");
    // A write outside the lock's scope: the hub refuses it, in the core's words.
    let e = control.set_output_gain(1, 1.0).unwrap_err();
    // -- WP MERGEP1D -- the hub holds the lock: its notice row.
    assert_eq!(code_of(&e), "hub", "{e:?}");
    assert_eq!(e.notice_key(), "installer_lock.hub");
    // The console's PIN does not unlock it; wrong PINs are the hub's.
    let e = control.installer_unlock("1357".into()).unwrap_err();
    assert_eq!(code_of(&e), "wrong_pin", "{e:?}");
    assert_eq!(control.installer_lock_state().wrong_tries, 1);
    // The hub's PIN does: one prompt.
    let st = control.installer_unlock(HUB_PIN.into()).unwrap();
    assert!(st.unlocked && !st.locked && st.installer_open, "{st:?}");
    assert!(control.set_output_gain(1, 1.0).is_ok());
    // Within the hub's limits still: the limit is named (F5).
    let e = control.set_output_gain(0, 6.0).unwrap_err();
    assert!(
        matches!(&e, DspiError::LimitExceeded { code, .. } if code == "out_gain"),
        "{e:?}"
    );
    assert_eq!(e.notice_key(), "limit_exceeded");
    // A whole-state write from a Control client: refused by name (F5).
    let doc = control.config_export("x".into(), "t".into(), 0).unwrap();
    // -- WP P1TZ -- with its token, so the refusal is the hub's.
    let token = control.confirm_token(dspi_core::ConfirmOp::ConfigImport);
    let r: Value = serde_json::from_str(&control.config_import_confirmed(
        doc,
        dspi_core::consts::IMPORT_ALL_BLOCKS,
        u32::MAX,
        Some(token),
    ))
    .unwrap();
    assert_eq!(r["noticeKey"], "image_refused", "{r}");
    // Locking again goes to the hub.
    let st = control.installer_lock().unwrap();
    assert!(st.locked && st.held_by == "hub", "{st:?}");
    assert_eq!(
        code_of(&control.set_output_gain(1, 0.0).unwrap_err()),
        "hub" // -- WP MERGEP1D --
    );
    // The hub's refusal window, as the console shows it.
    for _ in 0..=dspi_session::limits::LOCK_FREE_TRIES {
        let _ = control.installer_unlock("0000".into());
    }
    let e = control.installer_unlock("0000".into()).unwrap_err();
    assert_eq!(code_of(&e), "wait", "{e:?}");
    let st = control.installer_lock_state();
    assert!(st.retry_after_ms > 0, "{st:?}");

    // The Admin is never locked on the hub: its console shows it unlocked.
    let st = admin.installer_lock_refresh().unwrap();
    assert!(st.held_by == "hub" && st.unlocked && !st.locked, "{st:?}");

    // The hub's PIN cleared: the console's own lock is the one again.
    let mut f = serde_json::Map::new();
    f.insert(
        "handle".into(),
        json!(admin.link_devices().remove(0).handle),
    );
    f.insert("pin".into(), Value::Null);
    admin.link_hub_request("limits.pin", f).unwrap();
    let st = control.installer_lock_refresh().unwrap();
    assert_eq!(st.held_by, "console", "{st:?}");
    assert!(st.locked);
}
