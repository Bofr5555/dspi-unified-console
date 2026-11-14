//! WP HUBADMIN: the core's typed calls for the hub's admin API over DSPi Link
//! (`limits.get`/`set`, `limits.pin` with a recovery code, `mode.pin`) and the
//! hub's `limits.changed`/`limits.held`/`mode.changed` events through
//! `dspi-link`'s message hook. The client is the real one (`DspiCore` over
//! DSPi Link), the server the real one: a Control client paired with the
//! hub's PIN (pairing gives Control at most) and an Admin beside it.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::hub_admin::{COPY_HUB, NOTICE_HUB_COPY, NOTICE_HUB_ROLE, NOTICE_NO_LIMITS};
use dspi_core::{DspiCore, DspiError, HubAdminEvent, HubAdminListener, HubModePin};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);
const PIN: &str = "hub-7531";

fn hub_in(dir: &std::path::Path) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.to_owned())
        }),
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process("HUBADMIN hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial("HA-1")), 0)
        .unwrap();
    hub
}

fn scratch() -> std::path::PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hubadmin-link").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A core paired on `hub` with the hub's PIN (Control: the cap), given the
/// Admin role on the hub's machine when `admin` (`uconsole hub role <cid>
/// admin`; the consoles ask for control when they pair), bound to the device.
fn client(hub: &Hub, admin: bool) -> Arc<DspiCore> {
    let net = hub.network().unwrap();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
        "wp-hubadmin".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let window = net.auth().begin_pairing(Instant::now()).unwrap();
    core.link_pair(window.pin).unwrap();
    if admin {
        let cid = net.auth().clients().iter().map(|c| c.cid).max().unwrap();
        assert!(net.set_role(cid, Role::Admin));
    }
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

#[derive(Default)]
struct Heard(Mutex<Vec<HubAdminEvent>>);

impl HubAdminListener for Heard {
    fn on_hub_admin_event(&self, event: HubAdminEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Heard {
    fn wait_for(&self, kind: &str) -> HubAdminEvent {
        let mut found = None;
        wait_until(kind, || {
            found = self
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|e| e.kind == kind)
                .cloned();
            found.is_some()
        });
        self.0.lock().unwrap().clear();
        found.unwrap()
    }
}

fn profile(max_gain_db: f64, pages: &[&str]) -> String {
    json!({
        "schemaVersion": 1,
        "meta": {"kind": "limits", "name": "Limits profile"},
        "limits": {"outputs": [{"output": 0, "maxGainDb": max_gain_db}], "lockedPages": pages}
    })
    .to_string()
}

#[test]
fn wp_hubadmin_an_admin_edits_the_hubs_copy_and_a_control_client_reads_it() {
    let dir = scratch();
    let hub = hub_in(&dir);
    let control = client(&hub, false);
    let admin = client(&hub, true);
    // The hub's PIN pairs Control at most.
    assert_eq!(control.link_hub_role(), "control");
    // The Admin role was given live, on the hub's machine: the hub's answer
    // says so, and the console's target follows it after one read.
    assert!(admin.hub_limits_get().unwrap().can_edit);

    // Which copy each page edits, and whether it may.
    let t = control.installer_target();
    assert_eq!(
        (t.copy.as_str(), t.editable, t.notice_key.as_str()),
        (COPY_HUB, false, NOTICE_HUB_COPY),
        "{t:?}"
    );
    assert_eq!(t.read_only_key, NOTICE_HUB_ROLE);
    assert_eq!(t.role, "control");
    let t = admin.installer_target();
    assert!(
        t.editable && t.copy == COPY_HUB && t.read_only_key.is_empty(),
        "{t:?}"
    );

    let heard = Arc::new(Heard::default());
    control.set_hub_admin_listener(Some(heard.clone()));

    // Nothing kept yet; every role may read.
    let st = control.hub_limits_get().unwrap();
    assert!(
        !st.can_edit && !st.pin_set && st.profile_document.is_empty(),
        "{st:?}"
    );
    assert_eq!(
        st.pinned_modes,
        [HubModePin {
            role: "control".into(),
            mode: String::new()
        }]
    );

    // Below Admin: refused here, and on the hub too.
    let e = control.hub_limits_set(profile(3.0, &[])).unwrap_err();
    assert!(matches!(e, DspiError::Denied), "{e:?}");
    assert!(matches!(
        control.hub_pin_set(PIN.into()).unwrap_err(),
        DspiError::Denied
    ));
    assert!(matches!(
        control
            .hub_mode_pin("control".into(), "simple".into())
            .unwrap_err(),
        DspiError::Denied
    ));
    let mut f = serde_json::Map::new();
    f.insert(
        "handle".into(),
        json!(control.link_devices().remove(0).handle),
    );
    f.insert("pin".into(), json!(PIN));
    let e = control.link_hub_request("limits.pin", f).unwrap_err();
    assert!(
        matches!(&e, DspiError::Link { code, .. } if code == "denied"),
        "{e:?}"
    );

    // The Admin sets a profile with the device above it: held at once.
    admin.set_output_gain(0, 6.0).unwrap();
    let set = admin.hub_limits_set(profile(3.0, &["eq"])).unwrap();
    assert!(!set.held.is_empty(), "{set:?}");
    assert!(set.state.can_edit && !set.state.profile_document.is_empty());
    // The Control console hears of it, its lock read again.
    let ev = heard.wait_for("limits.changed");
    assert_eq!(ev.serial, "HA-1");
    let ev_held = {
        // limits.held may come before or after limits.changed
        let st = control.hub_limits_get().unwrap();
        assert!(!st.held.is_empty(), "{st:?}");
        st
    };
    assert!(!ev_held.pin_set);
    let doc: Value =
        serde_json::from_str(&control.hub_limits_get().unwrap().profile_document).unwrap();
    assert_eq!(doc["limits"]["outputs"][0]["maxGainDb"], json!(3.0));
    // The hub's limit holds for the Control client's writes.
    let e = control.set_output_gain(0, 5.0).unwrap_err();
    assert!(matches!(&e, DspiError::LimitExceeded { .. }), "{e:?}");

    // A PIN on the hub: a recovery code shown once; the lock is the hub's.
    let pin = admin.hub_pin_set(PIN.into()).unwrap();
    let code = pin.recovery_code.clone();
    assert_eq!(code.len(), 23, "{code}");
    assert_eq!(code.matches('-').count(), 3);
    assert!(pin.state.pin_set && pin.state.locked);
    assert!(pin.state.unlocked, "an Admin is never locked");
    let ev = heard.wait_for("limits.changed");
    assert!(ev.lock.pin_set && ev.lock.locked, "{:?}", ev.lock);
    assert_eq!(ev.lock.held_by, "hub");
    assert!(control.installer_lock_state().locked);
    // The hub never gives the code back.
    let text = std::fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(!text.contains(&code) && !text.contains(&code.replace('-', "")));
    assert!(!text.contains(PIN));
    assert!(
        serde_json::from_str::<Value>(&text).unwrap()["device_pin_recovery"]["HA-1"]["hash"]
            .is_string()
    );
    // The recovery code unlocks the Control console, however it is typed.
    let typed = code.to_lowercase().replace('-', " ");
    let st = control.installer_unlock(typed).unwrap();
    assert!(st.unlocked && !st.locked, "{st:?}");
    control.installer_lock().unwrap();
    // Wrong PINs: the hub's count (3 free, then a window).
    for _ in 0..dspi_session::limits::LOCK_FREE_TRIES {
        let _ = control.installer_unlock("0000".into());
    }
    let _ = control.installer_unlock("0000".into());
    let st = control.hub_limits_get().unwrap();
    assert!(st.retry_after_ms > 0 && st.wrong_tries >= 4, "{st:?}");

    // A pinned mode, hub-wide: the Admin sees every role's.
    let st = admin
        .hub_mode_pin("control".into(), "simple".into())
        .unwrap();
    assert_eq!(st.pinned_modes.len(), 3);
    assert!(
        st.pinned_modes
            .iter()
            .any(|p| p.role == "control" && p.mode == "simple")
    );
    let ev = heard.wait_for("mode.changed");
    assert_eq!((ev.role.as_str(), ev.mode.as_str()), ("control", "simple"));
    assert_eq!(ev.lock.hub_pinned_mode, "simple");
    assert_eq!(control.mode_pinned(), "simple");
    let st = admin.hub_mode_pin("control".into(), String::new()).unwrap();
    assert!(st.pinned_modes.iter().all(|p| p.mode.is_empty()), "{st:?}");
    heard.wait_for("mode.changed");

    // The PIN cleared: the lock is gone, so is the code.
    let st = admin.hub_pin_clear().unwrap();
    assert!(!st.pin_set && !st.locked);
    let ev = heard.wait_for("limits.changed");
    assert!(!ev.lock.locked, "{:?}", ev.lock);
    let text = std::fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(serde_json::from_str::<Value>(&text).unwrap()["device_pin_recovery"].is_null());
    // The profile cleared: nothing kept.
    let st = admin.hub_limits_clear().unwrap();
    assert!(st.profile_document.is_empty());
    heard.wait_for("limits.changed");
    assert!(control.set_output_gain(0, 5.0).is_ok());

    // Polled as well as told (the web's way).
    assert!(!control.hub_admin_events().is_empty());
    control.set_hub_admin_listener(None);
}

#[test]
fn wp_hubadmin_the_consoles_copy_over_a_hub_is_not_the_one_edited() {
    let dir = scratch();
    let hub = hub_in(&dir);
    let admin = client(&hub, true);
    // This console's own copy stays out of it: the hub's is set.
    admin.hub_limits_set(profile(3.0, &[])).unwrap();
    assert!(
        !admin.limits_status().active,
        "the console's own copy is untouched"
    );
    // Off the hub, the same device would be edited on the console: the
    // demo stands in for USB.
    let usb = DspiCore::new();
    usb.open_demo().unwrap();
    let t = usb.installer_target();
    assert_eq!(t.copy, "console");
    assert_eq!(
        usb.hub_limits_get().unwrap_err().notice_key(),
        NOTICE_NO_LIMITS
    );
}
