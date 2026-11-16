//! WP HB6 tests: the hub's C functions over real sockets on loopback, with `dspi-capi`'s
//! own Link functions as the client, on the demo device. No USB: `HUB_OPT_USB` is only
//! looked at, through what it configures.
//!
//! Two tests re-run themselves as a process of their own ([`in_a_process_of_its_own`]),
//! to have a logger and a thread list that no other test shares.
#![cfg(feature = "link")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use serde_json::Value;

use super::*;
use crate::fns_device::{
    dspi_core_free, dspi_core_new, dspi_is_connected, dspi_scan_devices, dspi_select_device,
    dspi_set_preamp,
};
use crate::fns_link::link::{
    LINK_EVENT_TOKEN_ISSUED, dspi_link_close, dspi_link_connect, dspi_link_pair,
    dspi_link_select_device, dspi_link_set_listener, dspi_link_status, dspi_link_ws_url,
};
use crate::logger_builder;
use crate::types::DeviceInfo;

const WAIT: Duration = Duration::from_secs(10);
const LOOPBACK: u32 = HUB_OPT_LOOPBACK;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// A scratch directory: `temp_dir` is under the build cache in the build container.
fn scratch(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-hb6-unit").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Call a function that takes a buffer, with room for any reply.
fn json(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 64 * 1024];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(
        needed > 0 && needed as usize <= buf.len(),
        "needed {needed}"
    );
    serde_json::from_slice(&buf[..needed as usize - 1]).unwrap()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// The handle, freed at the end of a test.
struct TestHub(*mut FfiHub);

impl TestHub {
    fn new() -> Self {
        let h = dspi_hub_new();
        assert!(!h.is_null());
        Self(h)
    }

    fn start_in(&self, dir: &std::path::Path, access: u8, options: u32) -> Value {
        let (name, store) = (c("Test hub"), c(dir.to_str().unwrap()));
        json(|b, l| {
            dspi_hub_start(
                self.0,
                name.as_ptr(),
                store.as_ptr(),
                0,
                access,
                options,
                b,
                l,
            )
        })
    }

    /// Started with PIN access and a demo device; returns its port.
    fn demo(&self, dir: &std::path::Path) -> u16 {
        let r = self.start_in(dir, HUB_ACCESS_PIN, LOOPBACK);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(dspi_hub_attach_demo(self.0), 0);
        r["port"].as_u64().unwrap() as u16
    }

    fn status(&self) -> Value {
        json(|b, l| dspi_hub_status(self.0, b, l))
    }

    fn clients(&self) -> Value {
        json(|b, l| dspi_hub_clients(self.0, b, l))
    }

    fn devices(&self) -> Value {
        json(|b, l| dspi_hub_devices(self.0, b, l))
    }

    fn pin(&self) -> String {
        let r = json(|b, l| dspi_hub_begin_pairing(self.0, b, l));
        assert_eq!(r["ok"], true, "{r}");
        r["pin"].as_str().unwrap().to_owned()
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        dspi_hub_free(self.0);
    }
}

/// A client, through the C ABI's own Link functions.
struct Client(*mut FfiCore);

impl Client {
    fn new() -> Self {
        Self(dspi_core_new())
    }

    fn connect(&self, port: u16, token: &str) -> Value {
        let host = c("127.0.0.1");
        let url = {
            let mut buf = vec![0u8; 256];
            let n = dspi_link_ws_url(
                host.as_ptr(),
                port,
                null(),
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            );
            String::from_utf8(buf[..n as usize - 1].to_vec()).unwrap()
        };
        let (url, name, ver, token, none) = (c(&url), c("phone"), c("1"), c(token), c(""));
        json(|b, l| {
            dspi_link_connect(
                self.0,
                url.as_ptr(),
                name.as_ptr(),
                ver.as_ptr(),
                token.as_ptr(),
                none.as_ptr(),
                b,
                l,
            )
        })
    }

    fn pair(&self, pin: &str) -> Value {
        let pin = c(pin);
        json(|b, l| dspi_link_pair(self.0, pin.as_ptr(), b, l))
    }

    fn status(&self) -> Value {
        json(|b, l| dspi_link_status(self.0, b, l))
    }

    /// Paired with this hub; returns the token.
    fn paired(&self, hub: &TestHub, port: u16) -> String {
        let hello = self.connect(port, "");
        assert_eq!(hello["needsPairing"], true, "{hello}");
        let r = self.pair(&hub.pin());
        assert_eq!(r["ok"], true, "{r}");
        r["token"].as_str().unwrap().to_owned()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        dspi_core_free(self.0);
    }
}

/// What a test has the callback do: called on the event thread, with the event.
type OnEvent = Box<dyn FnMut(u8, *mut FfiHub) + Send>;

/// What a callback saw, and what it does when called.
#[derive(Default)]
struct Seen {
    events: Mutex<Vec<(u8, i32, String)>>,
    hub: AtomicPtr<FfiHub>,
    on_event: Mutex<Option<OnEvent>>,
}

impl Seen {
    fn events(&self) -> Vec<(u8, i32, String)> {
        self.events.lock().unwrap().clone()
    }

    fn kinds(&self) -> Vec<u8> {
        self.events().into_iter().map(|e| e.0).collect()
    }
}

extern "C" fn record(event: u8, code: i32, text: *const c_char, user_data: *mut c_void) {
    // SAFETY: the tests pass a `Seen` that outlives the listener.
    let seen = unsafe { &*user_data.cast::<Seen>() };
    // SAFETY: the ABI's promise: never null, valid during the call.
    let text = unsafe { CStr::from_ptr(text) }.to_str().unwrap().to_owned();
    seen.events.lock().unwrap().push((event, code, text));
    if let Some(f) = seen.on_event.lock().unwrap().as_mut() {
        f(event, seen.hub.load(Ordering::SeqCst));
    }
}

fn listen(hub: &TestHub, seen: &Arc<Seen>) {
    seen.hub.store(hub.0, Ordering::SeqCst);
    let ud = Arc::as_ptr(seen).cast_mut().cast::<c_void>();
    assert!(dspi_hub_set_listener(hub.0, Some(record), ud));
}

/// Run `test` in a process of its own, for a logger and a thread list no other test
/// shares: this test binary, this one test, with `DSPI_HB6_CHILD` set.
fn in_a_process_of_its_own(name: &str, test: impl FnOnce()) {
    if std::env::var_os("DSPI_HB6_CHILD").is_some() {
        test();
        return;
    }
    let module = module_path!().split_once("::").unwrap().1;
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("{module}::{name}"), "--nocapture"])
        .env("DSPI_HB6_CHILD", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("1 passed"),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A hub's threads in this process that have not ended. Linux only: elsewhere, nothing
/// to read.
///
/// -- WP HUBFLAKE2 -- A joined thread can still be listed for a moment: `join` returns
/// when the kernel wakes it from the thread's exit, and the task leaves `/proc` a little
/// later. By then its own code is over and the kernel has marked it `PF_EXITING`, which
/// a thread left running never is, so those are not counted.
fn hub_threads() -> Vec<String> {
    /// `PF_EXITING`, in the flags of `/proc/<pid>/task/<tid>/stat`.
    const EXITING: u64 = 0x4;
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    tasks
        .filter_map(|t| {
            let path = t.ok()?.path();
            let name = std::fs::read_to_string(path.join("comm")).ok()?;
            let stat = std::fs::read_to_string(path.join("stat")).ok()?;
            // After the name in parentheses: state, ppid, pgrp, session, tty, tpgid, flags.
            let flags: u64 = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(6)?
                .parse()
                .ok()?;
            (flags & EXITING == 0).then(|| name.trim().to_owned())
        })
        .filter(|name| name.starts_with("hub-"))
        .collect()
}

// ── Start, stop, settings ─────────────────────────────────────────

#[test]
fn wp_hb6_a_demo_hub_starts_reports_and_stops() {
    let dir = scratch("start");
    let hub = TestHub::new();
    assert_eq!(
        hub.status(),
        serde_json::json!({ "ok": true, "running": false })
    );
    assert_eq!(hub.devices()["value"], serde_json::json!([]));
    assert_eq!(hub.clients()["code"], "not_running");

    let r = hub.start_in(&dir, HUB_ACCESS_PIN, LOOPBACK);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["name"], "Test hub");
    assert_eq!(r["access"], "pin");
    assert_eq!(r["tokensRevoked"], 0);
    let port = r["port"].as_u64().unwrap();
    assert!(port > 0, "the OS chose a port");
    let id = r["hubId"].as_str().unwrap().to_owned();

    assert_eq!(dspi_hub_attach_demo(hub.0), 0);
    assert_eq!(dspi_hub_attach_demo(hub.0), -1, "there is one demo device");
    let status = hub.status();
    assert_eq!(status["running"], true);
    assert_eq!(status["hubId"], id.as_str());
    assert_eq!(status["port"], port);
    assert_eq!(status["pairingOpen"], false);
    assert_eq!(status["pairingExpiresInMs"], 0);
    let devices = hub.devices();
    assert_eq!(devices["value"].as_array().unwrap().len(), 1, "{devices}");
    assert_eq!(devices["value"][0]["handle"], 0);
    assert_eq!(devices["value"][0]["state"], "online");
    assert!(!devices["value"][0]["serial"].as_str().unwrap().is_empty());
    assert_eq!(hub.clients()["value"], serde_json::json!([]));

    assert!(dspi_hub_stop(hub.0));
    assert_eq!(hub.status()["running"], false);
    assert!(dspi_hub_stop(hub.0), "stopping twice is fine");

    // The same handle, the same identity, the name kept.
    let empty = c("");
    let store = c(dir.to_str().unwrap());
    let again = json(|b, l| {
        dspi_hub_start(
            hub.0,
            empty.as_ptr(),
            store.as_ptr(),
            0,
            HUB_ACCESS_PIN,
            LOOPBACK,
            b,
            l,
        )
    });
    assert_eq!(again["hubId"], id.as_str());
    assert_eq!(again["name"], "Test hub");
}

#[test]
fn wp_hb6_a_hub_with_no_name_gets_the_default_and_keeps_it_unstored() {
    let dir = scratch("default-name");
    let hub = TestHub::new();
    let (empty, store) = (c(" \t"), c(dir.to_str().unwrap()));
    let r =
        json(|b, l| dspi_hub_start(hub.0, empty.as_ptr(), store.as_ptr(), 0, 0, LOOPBACK, b, l));
    assert_eq!(r["name"], DEFAULT_HUB_NAME);
    let saved = json(|b, l| dspi_hub_saved_settings(store.as_ptr(), b, l));
    assert_eq!(saved["name"], Value::Null);
}

#[test]
fn wp_hb6_a_second_start_is_refused_and_leaves_the_first_running() {
    let hub = TestHub::new();
    let port = hub.demo(&scratch("second"));
    let r = hub.start_in(&scratch("second-other"), HUB_ACCESS_PIN, LOOPBACK);
    assert_eq!(r["code"], "running");
    assert_eq!(hub.status()["port"], port);
}

#[test]
fn wp_hb6_a_port_in_use_is_an_error_and_nothing_is_left_running() {
    let first = TestHub::new();
    let port = first.demo(&scratch("port-a"));
    let hub = TestHub::new();
    let (name, store) = (c("x"), c(scratch("port-b").to_str().unwrap()));
    let r = json(|b, l| {
        dspi_hub_start(
            hub.0,
            name.as_ptr(),
            store.as_ptr(),
            port,
            0,
            LOOPBACK,
            b,
            l,
        )
    });
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "io");
    assert!(!r["error"].as_str().unwrap().is_empty());
    assert_eq!(hub.status()["running"], false);
}

#[test]
fn wp_hb6_bad_arguments_are_refused_before_anything_starts() {
    let hub = TestHub::new();
    let dir = scratch("bad");
    let (name, store, empty) = (c("x"), c(dir.to_str().unwrap()), c(""));
    let start = |name: *const c_char, store: *const c_char, access, options| {
        json(|b, l| dspi_hub_start(hub.0, name, store, 0, access, options, b, l))
    };
    for r in [
        start(null(), store.as_ptr(), 0, LOOPBACK),
        start(name.as_ptr(), null(), 0, LOOPBACK),
        start(name.as_ptr(), empty.as_ptr(), 0, LOOPBACK),
        start(name.as_ptr(), store.as_ptr(), 2, LOOPBACK),
        // -- review secb -- 8 is HUB_OPT_TLS_ON now: an unknown bit
        start(name.as_ptr(), store.as_ptr(), 0, 64),
    ] {
        assert_eq!(r["code"], "invalid", "{r}");
    }
    let bad = [b'a', 0xFF, 0];
    let r = start(bad.as_ptr().cast(), store.as_ptr(), 0, LOOPBACK);
    assert_eq!(r["code"], "invalid", "{r}");
    assert_eq!(hub.status()["running"], false);
    assert!(!dir.exists(), "nothing was created");
}

#[test]
fn wp_hb6_options_decide_the_bus_the_bind_and_the_first_scan() {
    let args = |options| StartArgs {
        name: "x".into(),
        store_dir: PathBuf::from("/nowhere"),
        port: 7,
        access: AccessMode::Open,
        options,
    };
    let plain = config_for(&args(0), "x".into());
    assert!(plain.bus.is_none());
    assert!(!plain.initial_scan, "never before the listener is in place");
    let network = plain.network.as_ref().unwrap();
    assert_eq!((network.port, network.access), (7, AccessMode::Open));
    assert!(network.bind.is_unspecified(), "every interface by default");
    assert!(!network.web, "no web bundle until WP HB5");

    let usb = config_for(&args(HUB_OPT_USB), "x".into());
    assert!(usb.bus.is_some());
    assert!(!usb.initial_scan);
    let looped = config_for(&args(HUB_OPT_LOOPBACK), "x".into());
    assert!(looped.network.unwrap().bind.is_loopback());

    assert!(scans_at_start(HUB_OPT_USB));
    assert!(!scans_at_start(0));
    assert!(!scans_at_start(HUB_OPT_APP));
    assert!(
        !scans_at_start(HUB_OPT_USB | HUB_OPT_APP),
        "with an app, nothing is claimed before it has let go"
    );
}

#[test]
fn wp_hb6_a_hub_name_is_cleaned() {
    assert_eq!(clean_hub_name("  Kitchen\u{7} \n"), "Kitchen");
    assert_eq!(clean_hub_name("\u{7}\n"), "");
    assert_eq!(
        clean_hub_name(&"é".repeat(100)).chars().count(),
        MAX_HUB_NAME
    );
}

// ── Access mode ───────────────────────────────────────────────────

#[test]
fn wp_hb6_settings_read_nothing_into_existence() {
    let dir = scratch("settings-none");
    let store = c(dir.to_str().unwrap());
    let saved = json(|b, l| dspi_hub_saved_settings(store.as_ptr(), b, l));
    assert_eq!(saved["ok"], true);
    assert_eq!(saved["exists"], false);
    assert_eq!(saved["hubId"], Value::Null);
    assert_eq!(saved["access"], Value::Null);
    assert_eq!(saved["defaultPort"], dspi_hub::DEFAULT_PORT);
    assert_eq!(saved["tokensRevokedIfOpen"], 0);
    assert!(!dir.exists(), "reading settings created {}", dir.display());

    assert_eq!(
        json(|b, l| dspi_hub_saved_settings(null(), b, l))["code"],
        "invalid"
    );

    // A file `load` would set aside reads as nothing, and is still there afterwards.
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hub-store.json");
    std::fs::write(&file, b"not json").unwrap();
    let saved = json(|b, l| dspi_hub_saved_settings(store.as_ptr(), b, l));
    assert_eq!(saved["exists"], false, "{saved}");
    assert_eq!(std::fs::read(&file).unwrap(), b"not json");
}

/// The warning before, the count after: what a UI needs for a change of access mode.
#[test]
fn wp_hb6_a_change_of_access_mode_is_warned_of_and_reported() {
    let dir = scratch("access");
    let store = c(dir.to_str().unwrap());
    let hub = TestHub::new();
    let port = hub.demo(&dir);
    let client = Client::new();
    let token = client.paired(&hub, port);
    dspi_link_close(client.0);
    assert!(dspi_hub_stop(hub.0));

    let before = json(|b, l| dspi_hub_saved_settings(store.as_ptr(), b, l));
    assert_eq!(before["exists"], true);
    assert_eq!(before["access"], "pin");
    assert_eq!(before["name"], "Test hub");
    assert_eq!(before["tokensRevokedIfPin"], 0, "no change, no loss");
    assert_eq!(before["tokensRevokedIfOpen"], 1, "{before}");
    let clients = before["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0]["name"], "phone");
    assert_eq!(clients[0]["online"], false);
    assert!(
        !before.to_string().contains(&token),
        "the token is not in the store's listing"
    );

    // The warning changed nothing: same mode, the client stays, and its token works.
    let same = hub.start_in(&dir, HUB_ACCESS_PIN, LOOPBACK);
    assert_eq!(same["tokensRevoked"], 0);
    assert_eq!(hub.clients()["value"].as_array().unwrap().len(), 1);
    assert!(dspi_hub_stop(hub.0));

    let open = hub.start_in(&dir, HUB_ACCESS_OPEN, LOOPBACK);
    assert_eq!(open["access"], "none");
    assert_eq!(open["tokensRevoked"], 1, "{open}");
    assert_eq!(hub.status()["tokensRevoked"], 1);
    assert_eq!(hub.clients()["value"], serde_json::json!([]));
    assert_eq!(
        json(|b, l| dspi_hub_begin_pairing(hub.0, b, l))["code"],
        "open_access"
    );
    assert_eq!(hub.status()["pairingOpen"], false);
    assert!(dspi_hub_stop(hub.0));

    let after = json(|b, l| dspi_hub_saved_settings(store.as_ptr(), b, l));
    assert_eq!(after["access"], "none");
    assert_eq!(after["tokensRevokedIfPin"], 0, "no clients left to lose");

    // Back: the old token is no good, and no new loss.
    let back = hub.start_in(&dir, HUB_ACCESS_PIN, LOOPBACK);
    assert_eq!(back["tokensRevoked"], 0);
    let port = back["port"].as_u64().unwrap() as u16;
    let client = Client::new();
    let hello = client.connect(port, &token);
    assert_eq!(hello["tokenRefused"], true, "{hello}");
}

// ── Clients ───────────────────────────────────────────────────────

#[test]
fn wp_hb6_a_client_pairs_is_listed_changes_role_and_is_revoked() {
    let hub = TestHub::new();
    let seen = Arc::new(Seen::default());
    listen(&hub, &seen);
    let port = hub.demo(&scratch("pair"));

    let client = Client::new();
    let hello = client.connect(port, "");
    assert_eq!(hello["needsPairing"], true, "{hello}");
    assert_eq!(hello["auth"], "pin");

    let window = json(|b, l| dspi_hub_begin_pairing(hub.0, b, l));
    let pin = window["pin"].as_str().unwrap();
    assert_eq!(pin.len(), 6);
    assert!(window["expiresInMs"].as_u64().unwrap() > 100_000);
    let status = hub.status();
    assert_eq!(status["pairingOpen"], true);
    assert!(status["pairingExpiresInMs"].as_u64().unwrap() > 100_000);
    assert!(
        !status.to_string().contains(pin),
        "the status does not show the PIN"
    );

    assert_eq!(client.pair("000000x")["ok"], false);
    let paired = client.pair(pin);
    assert_eq!(paired["ok"], true, "{paired}");
    let token = paired["token"].as_str().unwrap().to_owned();
    assert_eq!(
        hub.status()["pairingOpen"],
        false,
        "a right PIN ends the window"
    );

    let clients = hub.clients();
    assert_eq!(clients["value"].as_array().unwrap().len(), 1, "{clients}");
    let listed = &clients["value"][0];
    assert_eq!(listed["name"], "phone");
    assert_eq!(listed["role"], "control");
    assert!(listed["created"].as_u64().unwrap() > 0);
    assert!(listed["lastSeen"].as_u64().unwrap() > 0);
    wait_until("the client's session", || {
        hub.clients()["value"][0]["online"] == true
    });
    let cid = listed["cid"].as_u64().unwrap() as u32;

    // A finished pairing reaches the listener, which is what a page refreshes on.
    wait_until("the session event", || {
        seen.kinds().contains(&HUB_EVENT_SESSION_OPENED)
    });

    // The role reaches the live session: the same write, before and after.
    let serial = hub.devices()["value"][0]["serial"]
        .as_str()
        .unwrap()
        .to_owned();
    let c_serial = c(&serial);
    let bound = json(|b, l| dspi_link_select_device(client.0, 0, c_serial.as_ptr(), b, l));
    assert_eq!(bound["ok"], true, "{bound}");
    assert!(dspi_set_preamp(client.0, -3.0), "a control client writes");
    let viewer = c("viewer");
    assert!(dspi_hub_set_role(hub.0, cid, viewer.as_ptr()));
    assert_eq!(hub.clients()["value"][0]["role"], "viewer");
    assert!(!dspi_set_preamp(client.0, -4.0), "a viewer does not");
    assert!(!dspi_hub_set_role(hub.0, cid + 1, viewer.as_ptr()));
    assert!(!dspi_hub_set_role(hub.0, cid, null()));
    let boss = c("boss");
    assert!(!dspi_hub_set_role(hub.0, cid, boss.as_ptr()));
    assert_eq!(hub.clients()["value"][0]["role"], "viewer");

    assert!(!dspi_hub_revoke(hub.0, cid + 1));
    assert!(dspi_hub_revoke(hub.0, cid));
    assert!(!dspi_hub_revoke(hub.0, cid), "already gone");
    assert_eq!(hub.clients()["value"], serde_json::json!([]));
    let start = Instant::now();
    loop {
        let s = client.status();
        if s["closeCode"] == 4002 && s["tokenRefused"] == true {
            break;
        }
        assert!(start.elapsed() < WAIT, "not closed: {s}");
        thread::sleep(Duration::from_millis(5));
    }

    let again = Client::new();
    assert_eq!(again.connect(port, &token)["tokenRefused"], true);

    // Nothing the ABI hands the app holds the token.
    for (_, _, text) in seen.events() {
        assert!(!text.contains(&token));
    }
}

#[test]
fn wp_hb6_cancelling_closes_the_window_and_a_new_pin_replaces_the_old() {
    let hub = TestHub::new();
    let port = hub.demo(&scratch("cancel"));
    let first = hub.pin();
    let second = hub.pin();
    assert_eq!(hub.status()["pairingOpen"], true);
    assert!(dspi_hub_cancel_pairing(hub.0));
    assert_eq!(hub.status()["pairingOpen"], false);
    assert_eq!(hub.status()["pairingExpiresInMs"], 0);

    let client = Client::new();
    client.connect(port, "");
    assert_eq!(client.pair(&second)["ok"], false, "cancelled");
    // A window with another PIN: the old ones stay dead.
    let third = hub.pin();
    if first != third {
        assert_eq!(client.pair(&first)["ok"], false, "replaced");
    }
}

#[test]
fn wp_hb6_without_a_hub_the_acting_functions_say_so() {
    let hub = TestHub::new();
    assert_eq!(
        json(|b, l| dspi_hub_begin_pairing(hub.0, b, l))["code"],
        "not_running"
    );
    assert!(!dspi_hub_cancel_pairing(hub.0));
    assert!(!dspi_hub_revoke(hub.0, 1));
    let role = c("viewer");
    assert!(!dspi_hub_set_role(hub.0, 1, role.as_ptr()));
    assert_eq!(dspi_hub_attach_demo(hub.0), -1);
    let core = Client::new();
    assert!(!dspi_hub_attach_app(hub.0, core.0));
}

// ── Failure values and short buffers (Q1's rules, CG1's for acting calls) ─────

#[test]
fn wp_hb6_a_null_handle_gives_the_failure_value() {
    let mut buf = [0u8; 8];
    let p = buf.as_mut_ptr().cast::<c_char>();
    let null_hub = null_mut::<FfiHub>();
    let s = c("x");
    assert_eq!(dspi_hub_status(null_hub, p, 8), 0);
    assert_eq!(dspi_hub_devices(null_hub, p, 8), 0);
    assert_eq!(dspi_hub_clients(null_hub, p, 8), 0);
    assert_eq!(dspi_hub_begin_pairing(null_hub, p, 8), 0);
    assert_eq!(
        dspi_hub_start(null_hub, s.as_ptr(), s.as_ptr(), 0, 0, 0, p, 8),
        0
    );
    assert!(!dspi_hub_stop(null_hub));
    assert!(!dspi_hub_cancel_pairing(null_hub));
    assert!(!dspi_hub_revoke(null_hub, 1));
    assert!(!dspi_hub_set_role(null_hub, 1, s.as_ptr()));
    assert!(!dspi_hub_set_listener(null_hub, None, null_mut()));
    assert_eq!(dspi_hub_attach_demo(null_hub), -1);
    assert!(!dspi_hub_attach_app(null_hub, null_mut()));
    dspi_hub_free(null_hub);
    assert_eq!(buf, [0u8; 8], "nothing written");

    let hub = TestHub::new();
    hub.demo(&scratch("null-core"));
    assert!(!dspi_hub_attach_app(hub.0, null_mut()));
}

#[test]
fn wp_hb6_a_short_buffer_acts_on_nothing() {
    let dir = scratch("short");
    let hub = TestHub::new();
    let (name, store) = (c("x"), c(dir.to_str().unwrap()));
    let mut buf = vec![0xAAu8; HUB_REPLY_MIN as usize];
    let p = buf.as_mut_ptr().cast::<c_char>();
    for (ptr, len) in [
        (null_mut(), 0),
        (null_mut(), HUB_REPLY_MIN),
        (p, 0),
        (p, HUB_REPLY_MIN - 1),
    ] {
        let needed = dspi_hub_start(
            hub.0,
            name.as_ptr(),
            store.as_ptr(),
            0,
            0,
            LOOPBACK,
            ptr,
            len,
        );
        assert_eq!(needed, HUB_REPLY_MIN);
        assert_eq!(hub.status()["running"], false, "no hub with len {len}");
        assert!(!dir.exists(), "no store with len {len}");
    }

    hub.demo(&dir);
    for (ptr, len) in [
        (null_mut(), 0),
        (null_mut(), HUB_REPLY_MIN),
        (p, 0),
        (p, HUB_REPLY_MIN - 1),
    ] {
        assert_eq!(dspi_hub_begin_pairing(hub.0, ptr, len), HUB_REPLY_MIN);
        assert_eq!(
            hub.status()["pairingOpen"],
            false,
            "no window, and no PIN lost, with len {len}"
        );
    }
    assert!(buf.iter().all(|b| *b == 0xAA), "nothing written");

    // A window opened with a big enough buffer survives a short one asking again.
    let pin = hub.pin();
    assert_eq!(dspi_hub_begin_pairing(hub.0, p, 1), HUB_REPLY_MIN);
    let client = Client::new();
    client.connect(hub.status()["port"].as_u64().unwrap() as u16, "");
    assert_eq!(client.pair(&pin)["ok"], true);
}

#[test]
fn wp_hb6_the_c_abis_length_rule_for_reads() {
    let hub = TestHub::new();
    hub.demo(&scratch("reads"));
    let needed = dspi_hub_status(hub.0, null_mut(), 0);
    assert!(needed > 2);
    let mut buf = vec![0xAAu8; needed as usize + 1];
    let p = buf.as_mut_ptr().cast::<c_char>();
    assert_eq!(dspi_hub_status(hub.0, p, needed - 1), needed);
    assert!(buf.iter().all(|b| *b == 0xAA), "nothing written");
    assert_eq!(dspi_hub_status(hub.0, p, needed), needed);
    assert_eq!(buf[needed as usize - 1], 0);
    assert_eq!(buf[needed as usize], 0xAA, "and nothing past it");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Reads never write past `len`, and a short buffer gets the whole length or nothing.
    #[test]
    fn proptest_wp_hb6_reads_respect_any_length(which in 0usize..3, len in 0u32..512) {
        let hub = dspi_hub_new();
        let f: [extern "C" fn(*mut FfiHub, *mut c_char, u32) -> u32; 3] =
            [dspi_hub_status, dspi_hub_devices, dspi_hub_clients];
        let mut buf = vec![0xAAu8; len as usize + 8];
        let needed = f[which](hub, buf.as_mut_ptr().cast(), len);
        prop_assert!(needed > 0);
        let written = if needed <= len { needed as usize } else { 0 };
        prop_assert!(buf[written..].iter().all(|b| *b == 0xAA));
        if written > 0 {
            prop_assert_eq!(buf[written - 1], 0);
            let parsed: Value = serde_json::from_slice(&buf[..written - 1]).unwrap();
            prop_assert!(parsed.get("ok").is_some());
        }
        dspi_hub_free(hub);
    }

    /// Whatever the buffer, an acting call under the minimum does not act.
    #[test]
    fn proptest_wp_hb6_begin_pairing_with_any_short_buffer_opens_nothing(len in 0u32..HUB_REPLY_MIN) {
        let hub = dspi_hub_new();
        let mut buf = vec![0xAAu8; len as usize + 8];
        prop_assert_eq!(dspi_hub_begin_pairing(hub, buf.as_mut_ptr().cast(), len), HUB_REPLY_MIN);
        prop_assert!(buf.iter().all(|b| *b == 0xAA));
        dspi_hub_free(hub);
    }
}

// ── The listener ──────────────────────────────────────────────────

#[test]
fn wp_hb6_events_come_in_order_and_stopped_is_the_last() {
    let hub = TestHub::new();
    let seen = Arc::new(Seen::default());
    listen(&hub, &seen);
    let port = hub.demo(&scratch("events"));
    wait_until("the device event", || {
        seen.kinds().contains(&HUB_EVENT_DEVICE_ONLINE)
    });
    let serial = hub.devices()["value"][0]["serial"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        seen.events()[0],
        (HUB_EVENT_DEVICE_ONLINE, 0, serial.clone())
    );

    let client = Client::new();
    client.paired(&hub, port);
    wait_until("the session", || {
        seen.kinds().contains(&HUB_EVENT_SESSION_OPENED)
    });
    dspi_link_close(client.0);
    wait_until("the session's end", || {
        seen.kinds().contains(&HUB_EVENT_SESSION_CLOSED)
    });

    assert!(dspi_hub_stop(hub.0));
    // Stop has joined the event thread, so this is every event there was.
    let kinds = seen.kinds();
    assert_eq!(kinds.last(), Some(&HUB_EVENT_STOPPED), "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == HUB_EVENT_STOPPED).count(), 1);

    // It carries over to the next run, whose events follow.
    hub.demo(&scratch("events-2"));
    wait_until("the second run's device", || {
        seen.kinds()
            .iter()
            .skip(kinds.len())
            .any(|k| *k == HUB_EVENT_DEVICE_ONLINE)
    });
    assert!(dspi_hub_stop(hub.0));
    assert!(dspi_hub_set_listener(hub.0, None, null_mut()));
    let count = seen.events().len();
    hub.demo(&scratch("events-3"));
    assert!(dspi_hub_stop(hub.0));
    assert_eq!(
        seen.events().len(),
        count,
        "a cleared listener hears nothing"
    );
}

/// What the module's table says of the callback: reads work, the rest is refused, and
/// none of it hangs.
#[test]
fn wp_hb6_from_the_callback_reads_work_and_the_rest_is_refused() {
    let hub = TestHub::new();
    let seen = Arc::new(Seen::default());
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(Some(tx));
    *seen.on_event.lock().unwrap() = Some(Box::new(move |event, h| {
        if event != HUB_EVENT_DEVICE_ONLINE {
            return;
        }
        let Some(tx) = tx.lock().unwrap().take() else {
            return;
        };
        let status = json(|b, l| dspi_hub_status(h, b, l));
        let devices = json(|b, l| dspi_hub_devices(h, b, l));
        let clients = json(|b, l| dspi_hub_clients(h, b, l));
        let (name, store) = (c("x"), c("/nowhere"));
        let start = json(|b, l| dspi_hub_start(h, name.as_ptr(), store.as_ptr(), 0, 0, 0, b, l));
        let pairing = json(|b, l| dspi_hub_begin_pairing(h, b, l));
        let role = c("viewer");
        // Refused by these two as well as being a use-after-free if they were not.
        dspi_hub_free(h);
        let refused = (
            dspi_hub_stop(h),
            dspi_hub_set_listener(h, None, null_mut()),
            dspi_hub_attach_app(h, null_mut()),
            dspi_hub_attach_demo(h),
            dspi_hub_cancel_pairing(h),
            dspi_hub_revoke(h, 1),
            dspi_hub_set_role(h, 1, role.as_ptr()),
        );
        tx.send((status, devices, clients, start, pairing, refused))
            .unwrap();
    }));
    listen(&hub, &seen);
    hub.demo(&scratch("callback"));

    let (status, devices, clients, start, pairing, refused) = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(status["running"], true, "{status}");
    assert_eq!(devices["value"].as_array().unwrap().len(), 1);
    assert_eq!(clients["ok"], true);
    assert_eq!(start["code"], "in_callback");
    assert_eq!(pairing["code"], "in_callback");
    assert_eq!(refused, (false, false, false, -1, false, false, false));

    // All of that left the hub as it was, and the thread fit to call in again.
    assert_eq!(hub.status()["running"], true);
    assert_eq!(hub.status()["pairingOpen"], false);
    assert!(dspi_hub_stop(hub.0));
    assert_eq!(seen.kinds().last(), Some(&HUB_EVENT_STOPPED));
}

/// The trap of HUB.md section 3: a listener reading while the hub stops must not end up
/// with the last owner, and must not deadlock with the stop.
#[test]
fn wp_hb6_stop_during_a_listeners_reads_finishes() {
    let hub = TestHub::new();
    let seen = Arc::new(Seen::default());
    let reads = Arc::new(AtomicUsize::new(0));
    {
        let reads = Arc::clone(&reads);
        *seen.on_event.lock().unwrap() = Some(Box::new(move |_, h| {
            // Slow enough to be in here as the stop arrives.
            for _ in 0..20 {
                json(|b, l| dspi_hub_status(h, b, l));
                json(|b, l| dspi_hub_devices(h, b, l));
                reads.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    listen(&hub, &seen);
    let (tx, rx) = mpsc::channel();
    for round in 0..3 {
        let port = hub.demo(&scratch(&format!("stop-reads-{round}")));
        let client = Client::new();
        client.connect(port, "");
        wait_until("a read in progress", || reads.load(Ordering::SeqCst) > 0);
        let raw = hub.0 as usize;
        let tx = tx.clone();
        thread::spawn(move || {
            tx.send(dspi_hub_stop(raw as *mut FfiHub)).unwrap();
        });
        assert_eq!(rx.recv_timeout(WAIT), Ok(true), "the stop hung");
        assert_eq!(hub.status()["running"], false);
        assert_eq!(seen.kinds().last(), Some(&HUB_EVENT_STOPPED));
    }
}

#[test]
fn wp_hb6_replacing_the_listener_waits_for_a_call_in_progress() {
    let hub = TestHub::new();
    let seen = Arc::new(Seen::default());
    let (inside_tx, inside_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let inside_tx = Mutex::new(Some(inside_tx));
    let go_rx = Mutex::new(go_rx);
    *seen.on_event.lock().unwrap() = Some(Box::new(move |_, _| {
        if let Some(tx) = inside_tx.lock().unwrap().take() {
            tx.send(()).unwrap();
            go_rx.lock().unwrap().recv().unwrap();
        }
    }));
    listen(&hub, &seen);
    hub.demo(&scratch("replace"));
    inside_rx.recv_timeout(WAIT).unwrap();

    let raw = hub.0 as usize;
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        done_tx
            .send(dspi_hub_set_listener(raw as *mut FfiHub, None, null_mut()))
            .unwrap();
    });
    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "it returned with the call still running"
    );
    go_tx.send(()).unwrap();
    assert_eq!(done_rx.recv_timeout(WAIT), Ok(true));
}

/// A whole run with hub threads counted: nothing is left behind by a free, even after a
/// listener tried to free from inside and read on the way out.
#[test]
fn wp_hb6_free_leaves_no_hub_thread_behind() {
    in_a_process_of_its_own("wp_hb6_free_leaves_no_hub_thread_behind", || {
        let seen = Arc::new(Seen::default());
        *seen.on_event.lock().unwrap() = Some(Box::new(|_, h| {
            dspi_hub_free(h);
            assert!(!dspi_hub_stop(h));
            json(|b, l| dspi_hub_status(h, b, l));
        }));
        let hub = TestHub::new();
        listen(&hub, &seen);
        let port = hub.demo(&scratch("threads"));
        let client = Client::new();
        client.connect(port, "");
        if cfg!(target_os = "linux") {
            let threads = hub_threads();
            assert!(threads.iter().any(|t| t == "hub-events"), "{threads:?}");
        }
        drop(hub);
        assert_eq!(seen.kinds().last(), Some(&HUB_EVENT_STOPPED));
        assert_eq!(hub_threads(), Vec::<String>::new());
    });
}

// ── Secrets ───────────────────────────────────────────────────────

struct Capture(Mutex<Vec<String>>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        // All but what `logger_builder` holds back, which is tested on its own below.
        if !record.target().starts_with("tungstenite") || record.level() <= log::Level::Debug {
            self.0.lock().unwrap().push(format!(
                "{} {}: {}",
                record.level(),
                record.target(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

/// `RUST_LOG=trace` does not get the frames tungstenite would print whole, and still
/// gets everything else.
#[test]
fn wp_hb6_the_logger_holds_back_the_frame_dumps() {
    use log::{Level, Log, Metadata};
    let mut asked = env_logger::Builder::new();
    asked.parse_filters("trace");
    let logger = logger_builder(asked).build();
    let at =
        |target, level| logger.enabled(&Metadata::builder().target(target).level(level).build());
    assert!(!at("tungstenite::protocol", Level::Trace));
    assert!(!at("tungstenite::protocol::frame", Level::Trace));
    assert!(at("tungstenite::protocol", Level::Debug));
    assert!(at("dspi_hub::server", Level::Trace));

    let mut named = env_logger::Builder::new();
    named.parse_filters("tungstenite=trace");
    let logger = logger_builder(named).build();
    let meta = Metadata::builder()
        .target("tungstenite::protocol")
        .level(Level::Trace)
        .build();
    assert!(
        !logger.enabled(&meta),
        "asking for it by name changes nothing"
    );
}

extern "C" fn drop_link_event(
    _event: u8,
    _code: i32,
    _text: *const c_char,
    _token: *const c_char,
    _user_data: *mut c_void,
) {
    let _ = LINK_EVENT_TOKEN_ISSUED;
}

/// At the most talkative level, through a pairing, a reconnect with the token, a change
/// of role, a revocation and a stop, no line of ours has the token or the PIN. A process
/// of its own, since a logger can be set only once.
#[test]
fn wp_hb6_no_token_or_pin_reaches_a_log() {
    in_a_process_of_its_own("wp_hb6_no_token_or_pin_reaches_a_log", || {
        let capture: &'static Capture = Box::leak(Box::new(Capture(Mutex::new(Vec::new()))));
        log::set_logger(capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);

        let dir = scratch("logs");
        let hub = TestHub::new();
        let seen = Arc::new(Seen::default());
        listen(&hub, &seen);
        let port = hub.demo(&dir);
        let client = Client::new();
        assert!(dspi_link_set_listener(
            client.0,
            Some(drop_link_event),
            null_mut()
        ));
        client.connect(port, "");
        assert_eq!(client.pair("000000")["ok"], false);
        let pin = hub.pin();
        let paired = client.pair(&pin);
        let token = paired["token"].as_str().unwrap().to_owned();
        assert!(!token.is_empty());
        dspi_link_close(client.0);

        let again = Client::new();
        assert_eq!(again.connect(port, &token)["needsPairing"], false);
        let cid = hub.clients()["value"][0]["cid"].as_u64().unwrap() as u32;
        let role = c("viewer");
        assert!(dspi_hub_set_role(hub.0, cid, role.as_ptr()));
        assert!(dspi_hub_revoke(hub.0, cid));
        assert!(dspi_hub_stop(hub.0));

        // A mode change's log line, with a client to lose.
        hub.demo(&dir);
        let third = Client::new();
        third.paired(&hub, hub.status()["port"].as_u64().unwrap() as u16);
        assert!(dspi_hub_stop(hub.0));
        assert_eq!(
            hub.start_in(&dir, HUB_ACCESS_OPEN, LOOPBACK)["tokensRevoked"],
            1
        );
        assert!(dspi_hub_stop(hub.0));

        let lines = capture.0.lock().unwrap();
        assert!(
            lines.iter().any(|l| l.contains("every token is revoked")),
            "the capture works: {lines:#?}"
        );
        for line in lines.iter() {
            assert!(!line.contains(&token), "the token in a log: {line}");
            assert!(!line.contains(&pin), "the PIN in a log: {line}");
        }
        for (_, _, text) in seen.events() {
            assert!(!text.contains(&token) && !text.contains(&pin));
        }
    });
}

// ── The app on the hub ────────────────────────────────────────────

/// The app gets the hub's device through the broker, a network client shares it, and a
/// stop gives the app its direct path back without a hang.
#[test]
fn wp_hb6_the_app_shares_a_device_and_gets_it_back_at_stop() {
    let dir = scratch("app");
    let hub = TestHub::new();
    let r = hub.start_in(&dir, HUB_ACCESS_OPEN, LOOPBACK | HUB_OPT_APP);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(dspi_hub_attach_demo(hub.0), 0);
    let port = r["port"].as_u64().unwrap() as u16;
    let serial = hub.devices()["value"][0]["serial"]
        .as_str()
        .unwrap()
        .to_owned();

    let app = Client::new();
    assert!(dspi_hub_attach_app(hub.0, app.0));
    let second = Client::new();
    assert!(!dspi_hub_attach_app(hub.0, second.0), "one app per hub");

    let mut found = vec![DeviceInfo::default(); 4];
    let count = dspi_scan_devices(app.0, found.as_mut_ptr(), 4);
    assert_eq!(count, 1, "the app sees the hub's device");
    let c_serial = c(&serial);
    assert!(dspi_select_device(app.0, c_serial.as_ptr()));
    assert!(dspi_is_connected(app.0));

    let client = Client::new();
    let hello = client.connect(port, "");
    assert_eq!(hello["auth"], "none", "{hello}");
    wait_until("the client's socket", || {
        hub.status()["connections"].as_u64().unwrap() >= 1
    });

    let (tx, rx) = mpsc::channel();
    let raw = hub.0 as usize;
    thread::spawn(move || {
        tx.send(dspi_hub_stop(raw as *mut FfiHub)).unwrap();
    });
    assert_eq!(rx.recv_timeout(WAIT * 2), Ok(true), "the stop hung");
    assert!(
        !dspi_is_connected(app.0),
        "the hub's device left with the hub, and there is no direct one here"
    );
    wait_until("the client to be told", || {
        client.status()["closeCode"] == 4003 || client.status()["phase"] == "reconnecting"
    });

    // The core is free to attach to the next run.
    let r = hub.start_in(&dir, HUB_ACCESS_OPEN, LOOPBACK | HUB_OPT_APP);
    assert_eq!(r["ok"], true, "{r}");
    assert!(dspi_hub_attach_app(hub.0, app.0));
}

// ── The header ────────────────────────────────────────────────────

fn header() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/dspi_core.h"))
        .unwrap()
}

#[test]
fn wp_hb6_the_functions_are_in_the_header() {
    let h = header();
    for proto in [
        "typedef struct FfiHub FfiHub;",
        "struct FfiHub *dspi_hub_new(void)",
        "void dspi_hub_free(struct FfiHub *hub)",
        "bool dspi_hub_set_listener(struct FfiHub *hub,",
        "uint32_t dspi_hub_saved_settings(const char *store_dir, char *buf, uint32_t len)",
        "uint32_t dspi_hub_start(struct FfiHub *hub,",
        "bool dspi_hub_stop(struct FfiHub *hub)",
        "bool dspi_hub_attach_app(struct FfiHub *hub, struct FfiCore *core)",
        "int32_t dspi_hub_attach_demo(struct FfiHub *hub)",
        "uint32_t dspi_hub_status(struct FfiHub *hub, char *buf, uint32_t len)",
        "uint32_t dspi_hub_devices(struct FfiHub *hub, char *buf, uint32_t len)",
        "uint32_t dspi_hub_clients(struct FfiHub *hub, char *buf, uint32_t len)",
        "uint32_t dspi_hub_begin_pairing(struct FfiHub *hub, char *buf, uint32_t len)",
        "bool dspi_hub_cancel_pairing(struct FfiHub *hub)",
        "bool dspi_hub_revoke(struct FfiHub *hub, uint32_t cid)",
        "bool dspi_hub_set_role(struct FfiHub *hub, uint32_t cid, const char *role)",
        "#define HUB_REPLY_MIN 4096",
        "#define HUB_ACCESS_OPEN 1",
        "#define HUB_OPT_LOOPBACK 4",
        "#define HUB_EVENT_STOPPED 5",
        "HubEventCallback",
    ] {
        assert!(h.contains(proto), "{proto} is not in the header");
    }
}

/// A C compiler takes the header, and calling the functions type-checks.
#[test]
fn wp_hb6_the_header_compiles_as_c() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let out = PathBuf::from(env!("OUT_DIR")).join("wp_hb6_header.c");
    std::fs::write(
        &out,
        "#include <stdbool.h>\n#include <stdint.h>\n#include \"dspi_core.h\"\n\
         static void cb(uint8_t e, int32_t c, const char *t, void *u)\n\
         { (void)e; (void)c; (void)t; (void)u; }\n\
         bool f(struct FfiCore *core, char *b)\n{\n\
         struct FfiHub *h = dspi_hub_new();\n\
         HubEventCallback typed = cb;\n\
         dspi_hub_set_listener(h, typed, 0);\n\
         dspi_hub_start(h, \"n\", \"d\", 0, HUB_ACCESS_PIN, HUB_OPT_USB | HUB_OPT_APP, b, HUB_REPLY_MIN);\n\
         dspi_hub_attach_app(h, core);\n\
         return dspi_hub_stop(h);\n}\n",
    )
    .unwrap();
    let got = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Werror", "-fsyntax-only", "-I"])
        .arg(&dir)
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    std::fs::remove_file(&out).unwrap();
}

// -- review secb -- TLS through the C API's start options
#[test]
fn review_secb_the_tls_options_start_the_mode_asked_for() {
    assert_eq!(tls_for(0), (TlsMode::Off, false), "off unless asked");
    assert_eq!(tls_for(HUB_OPT_TLS_ON), (TlsMode::On, false));
    assert_eq!(tls_for(HUB_OPT_TLS_REQUIRE), (TlsMode::Require, false));
    assert_eq!(tls_for(HUB_OPT_UPSTREAM_COMPAT), (TlsMode::On, true));
    for bad in [
        HUB_OPT_TLS_REQUIRE | HUB_OPT_TLS_ON,
        HUB_OPT_TLS_REQUIRE | HUB_OPT_UPSTREAM_COMPAT,
        64,
    ] {
        let (name, store) = (c("x"), c("/nowhere"));
        assert!(
            parse_start(name.as_ptr(), store.as_ptr(), 0, HUB_ACCESS_PIN, bad).is_err(),
            "{bad}"
        );
    }
    let dir = scratch("secb-tls");
    let hub = TestHub::new();
    let r = hub.start_in(&dir, HUB_ACCESS_PIN, LOOPBACK | HUB_OPT_TLS_REQUIRE);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["tls"], "require", "{r}");
    assert_eq!(r["upstreamCompat"], false, "{r}");
    assert!(
        r["tlsFingerprint"].as_str().unwrap().starts_with("SHA256:"),
        "{r}"
    );
    assert!(dspi_hub_stop(hub.0));
    let r = hub.start_in(&dir, HUB_ACCESS_PIN, LOOPBACK);
    assert_eq!(
        (r["tls"].as_str(), r["tlsFingerprint"].is_null()),
        (Some("off"), true),
        "{r}"
    );
}
