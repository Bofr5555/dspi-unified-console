//! WP P1HL's done criteria, through a running hub with real worker threads and
//! its `hub-system` thread: a Control client's write above a limit is refused;
//! a Control client's bulk image on a limited or a locked device is refused, an
//! Admin's accepted (and held); a Control client of a locked device outside the
//! lock's scope is refused; a system scene above a limit is refused; an Admin's
//! profile change is accepted and a Control's refused; a pinned mode is
//! reported; the store keeps no plain PIN, and a diagnostics bundle carries no
//! PIN hash. Also: a slot load and a narrowed profile are read back, held and
//! reported (OD-P1j), and the hub's guarded revert passes a limit set during its
//! countdown.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::limits::{self, HubLimits, LimitsPersist, StoredLimits};
use dspi_hub::session::Pop;
use dspi_hub::store::HubStore;
use dspi_hub::{
    Completion, Hub, HubConfig, HubError, HubHandle, Outbound, Role, Session, apply_guard,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::ext::alink::AlinkRole;
use dspi_proto::generated::opcodes as op;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AlinkSetup;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(15);

fn hub() -> Hub {
    Hub::start(HubConfig {
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        ..HubConfig::in_process("p1hl")
    })
    .unwrap()
}

fn plug(hub: &Hub, serial: &str) -> u8 {
    hub.attach(Box::new(FakeDevice::demo().with_serial(serial)), 0)
        .unwrap()
}

/// A network client minus the socket.
struct Client {
    hub: HubHandle,
    session: Arc<Session>,
    handle: u8,
}

impl Client {
    fn open(hub: &Hub, handle: u8, role: Role) -> Self {
        let hub = hub.handle();
        let session = hub.open_session(role, None).unwrap();
        Self {
            hub,
            session,
            handle,
        }
    }

    fn command(&self, dir: Dir, code: u8, w_value: u16, len: u16, payload: Vec<u8>) -> CmdResponse {
        let (tx, rx) = mpsc::channel();
        let req = CmdRequest {
            tag: 1,
            handle: self.handle,
            dir,
            b_request: code,
            w_value,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: len,
            payload,
        };
        self.hub.submit(
            &self.session,
            req,
            Completion::new(1, move |r| {
                let _ = tx.send(r);
            }),
        );
        rx.recv_timeout(WAIT).unwrap()
    }

    fn set_f32(&self, code: u8, w_value: u16, v: f32) -> CmdResponse {
        self.command(Dir::Set, code, w_value, 4, v.to_le_bytes().to_vec())
    }

    fn get_f32(&self, code: u8, w_value: u16) -> f32 {
        let r = self.command(Dir::Get, code, w_value, 4, Vec::new());
        assert_eq!(r.status, status::OK);
        f32::from_le_bytes(r.payload[..4].try_into().unwrap())
    }

    fn gain(&self, o: u16) -> f32 {
        self.get_f32(op::REQ_GET_OUTPUT_GAIN, o)
    }

    fn set_gain(&self, o: u16, db: f32) -> CmdResponse {
        self.set_f32(op::REQ_SET_OUTPUT_GAIN, o, db)
    }

    fn image(&self) -> Vec<u8> {
        let r = self.command(Dir::Get, op::REQ_GET_ALL_PARAMS, 0, 8192, Vec::new());
        assert_eq!(r.status, status::OK);
        r.payload
    }

    fn msg(&self, msg: Value) -> Value {
        let text = limits::handle_message(&self.hub, &self.session, &msg)
            .expect("a limits.* or mode.* message is answered");
        serde_json::from_str(&text).unwrap()
    }

    fn set_profile(&self, profile: Value) -> Value {
        self.msg(json!({"t": "limits.set", "id": 1, "handle": self.handle, "profile": profile}))
    }

    fn get(&self) -> Value {
        self.msg(json!({"t": "limits.get", "id": 2, "handle": self.handle}))
    }

    /// The next event of type `t` this session hears.
    fn event(&self, t: &str) -> Value {
        let start = Instant::now();
        loop {
            if let Pop::Item(Outbound::Event(json)) =
                self.session.outbox().pop(Duration::from_millis(20))
            {
                let v: Value = serde_json::from_str(&json).unwrap();
                if v["t"] == t {
                    return v;
                }
            }
            assert!(start.elapsed() < WAIT, "no {t}");
        }
    }
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

/// The refusal's payload, which must be there.
fn refusal(r: &CmdResponse) -> Value {
    assert_eq!(r.status, status::DENIED, "{r:?}");
    serde_json::from_slice(&r.payload).unwrap()
}

/// Output 1's gain at most 3 dB, the master volume at most -10 dB.
fn gain_profile() -> Value {
    json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "masterVolumeMaxDb": -10.0})
}

#[test]
fn wp_p1hl_a_control_clients_write_above_a_limit_is_refused() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-A");
    let other = plug(&hub, "P1HL-A2");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let set = admin.set_profile(gain_profile());
    assert!(ok(&set), "{set}");
    let r = control.set_gain(0, 6.0);
    let why = refusal(&r);
    assert_eq!(why["refused"], "limit_exceeded");
    assert_eq!(why["limit"], "out_gain");
    assert!(
        why["msg"].as_str().unwrap().contains("limit of 3 dB"),
        "{why}"
    );
    assert_eq!(control.set_gain(0, 2.0).status, status::OK);
    assert_eq!(admin.gain(0), 2.0);
    // An Admin is held to the profile too; it changes the profile to go past it.
    assert_eq!(
        refusal(&admin.set_gain(0, 6.0))["refused"],
        "limit_exceeded"
    );
    assert_eq!(admin.gain(0), 2.0, "nothing reached the device");
    let vol = control.set_f32(op::REQ_SET_MASTER_VOLUME, 0, -3.0);
    assert_eq!(refusal(&vol)["limit"], "master_volume");
    assert_eq!(
        control.set_f32(op::REQ_SET_MASTER_VOLUME, 0, -128.0).status,
        status::OK,
        "muting is always allowed"
    );
    // Another output, another device: not limited.
    assert_eq!(control.set_gain(1, 6.0).status, status::OK);
    let free = Client::open(&hub, other, Role::Control);
    assert_eq!(free.set_gain(0, 6.0).status, status::OK);
    // Cleared: as before.
    assert!(ok(&admin.set_profile(Value::Null)));
    assert_eq!(control.set_gain(0, 6.0).status, status::OK);
}

#[test]
fn wp_p1hl_a_control_clients_bulk_image_is_refused_an_admins_accepted_and_held() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-B");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    // Without a profile a Control client may write an image (class control).
    assert_eq!(admin.set_gain(0, 10.0).status, status::OK);
    let loud = admin.image();
    assert_eq!(
        control
            .command(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, 0, loud.clone())
            .status,
        status::OK
    );
    assert_eq!(admin.set_gain(0, 0.0).status, status::OK);
    assert!(ok(&admin.set_profile(gain_profile())));
    let r = control.command(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, 0, loud.clone());
    assert_eq!(refusal(&r)["refused"], "image_refused");
    let chunk = control.command(
        Dir::Set,
        op::REQ_SET_ALL_PARAMS_CHUNK,
        0,
        0,
        loud[..64].to_vec(),
    );
    assert_eq!(refusal(&chunk)["refused"], "image_refused");
    assert_eq!(admin.gain(0), 0.0, "nothing reached the device");
    // An Admin's image is accepted, then held to the profile and reported.
    let watcher = Client::open(&hub, handle, Role::Viewer);
    let r = admin.command(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, 0, loud);
    assert_eq!(r.status, status::OK, "{r:?}");
    let held = watcher.event("limits.held");
    assert_eq!(held["serial"], "P1HL-B");
    let rows: Vec<&str> = held["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert!(
        rows.iter().any(|r| r.starts_with("out.gain[0] 10 → 3")),
        "{rows:?}"
    );
    assert_eq!(admin.gain(0), 3.0);
    assert_eq!(admin.get()["held"], held["rows"]);
}

#[test]
fn wp_p1hl_a_locked_devices_control_clients_get_only_the_locks_scope() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-C");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq", "routing"]);
    assert!(ok(&admin.set_profile(p)));
    let pin = admin.msg(json!({"t": "limits.pin", "id": 3, "handle": handle, "pin": "2468"}));
    assert!(ok(&pin), "{pin}");
    let st = control.get();
    assert_eq!(st["locked"], true, "{st}");
    assert_eq!(st["unlocked"], false);
    assert_eq!(st["pin_set"], true);
    assert!(
        !st.to_string().contains("salt") && !st.to_string().contains("hash"),
        "{st}"
    );
    // Volume, mute and preset pass.
    assert_eq!(
        control.set_f32(op::REQ_SET_MASTER_VOLUME, 0, -20.0).status,
        status::OK
    );
    assert_eq!(
        control
            .command(Dir::Set, op::REQ_SET_USER_MUTE, 0, 1, vec![1])
            .status,
        status::OK
    );
    assert_eq!(
        control
            .command(Dir::Get, op::REQ_PRESET_LOAD, 0, 1, Vec::new())
            .status,
        status::OK
    );
    // Anything else is refused, within the limits or not.
    assert_eq!(
        refusal(&control.set_gain(0, 1.0))["refused"],
        "installer_lock"
    );
    assert_eq!(
        refusal(&control.set_f32(op::REQ_SET_OUTPUT_DELAY, 0, 1.0))["refused"],
        "installer_lock"
    );
    // Reads stay open.
    let _ = control.gain(0);
    // The wrong PIN, then the right one: for this session only.
    let wrong =
        control.msg(json!({"t": "limits.unlock", "id": 4, "handle": handle, "pin": "1111"}));
    assert_eq!(wrong["code"], "bad_pin", "{wrong}");
    let right =
        control.msg(json!({"t": "limits.unlock", "id": 5, "handle": handle, "pin": "2468"}));
    assert!(ok(&right), "{right}");
    assert_eq!(control.set_gain(0, 1.0).status, status::OK);
    assert_eq!(
        refusal(&control.set_gain(0, 6.0))["refused"],
        "limit_exceeded",
        "unlocked is still within the limits"
    );
    let other = Client::open(&hub, handle, Role::Control);
    assert_eq!(
        refusal(&other.set_gain(0, 1.0))["refused"],
        "installer_lock"
    );
    // An Admin needs no PIN.
    assert_eq!(admin.set_gain(0, 2.0).status, status::OK);
    // Locking again.
    assert!(ok(
        &control.msg(json!({"t": "limits.lock", "id": 6, "handle": handle}))
    ));
    assert_eq!(
        refusal(&control.set_gain(0, 1.0))["refused"],
        "installer_lock"
    );
    // A locked device refuses a Control client's image too.
    let img = admin.image();
    let r = control.command(Dir::Set, op::REQ_SET_ALL_PARAMS, 0, 0, img);
    assert_eq!(refusal(&r)["refused"], "image_refused");
    // A Viewer may not unlock.
    let viewer = Client::open(&hub, handle, Role::Viewer);
    let v = viewer.msg(json!({"t": "limits.unlock", "id": 7, "handle": handle, "pin": "2468"}));
    assert_eq!(v["code"], "denied", "{v}");
}

#[test]
fn wp_p1hl_an_admins_profile_change_is_accepted_a_controls_refused() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-D");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let viewer = Client::open(&hub, handle, Role::Viewer);
    let r = control.set_profile(gain_profile());
    assert_eq!(r["code"], "denied", "{r}");
    let r = control.msg(json!({"t": "limits.pin", "id": 1, "handle": handle, "pin": "2468"}));
    assert_eq!(r["code"], "denied", "{r}");
    assert!(viewer.get()["profile"].is_null());
    // A limits document of the one family is taken as it is.
    let doc = json!({
        "schemaVersion": 1,
        "meta": {"kind": "limits"},
        "limits": {"serial": "somewhere else", "outputs": [{"output": 0, "maxGainDb": 3.0}]},
    });
    let set = admin.set_profile(doc);
    assert!(ok(&set), "{set}");
    assert_eq!(set["v"], 1);
    let seen = viewer.get();
    assert_eq!(seen["profile"]["serial"], "P1HL-D", "{seen}");
    assert_eq!(seen["profile"]["outputs"][0]["maxGainDb"], 3.0);
    let changed = viewer.event("limits.changed");
    assert_eq!(changed["serial"], "P1HL-D");
    let bad = admin.set_profile(json!("not json"));
    assert_eq!(bad["code"], "bad_request", "{bad}");
    let none = admin.msg(json!({"t": "limits.get", "id": 9, "handle": 200}));
    assert_eq!(none["code"], "no_device");
}

#[test]
fn wp_p1hl_a_pinned_mode_is_reported_to_a_client() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-E");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let r = control.msg(json!({"t": "mode.pinned", "id": 1}));
    assert!(ok(&r) && r["mode"].is_null(), "{r}");
    let r = control.msg(json!({"t": "mode.pin", "id": 2, "role": "control", "mode": "simple"}));
    assert_eq!(r["code"], "denied");
    let r = admin.msg(json!({"t": "mode.pin", "id": 3, "role": "control", "mode": "simple"}));
    assert!(ok(&r), "{r}");
    let r = admin.msg(json!({"t": "mode.pin", "id": 4, "role": "viewer", "mode": "expert"}));
    assert_eq!(r["code"], "bad_request");
    let ev = control.event("mode.changed");
    assert_eq!(ev["role"], "control");
    let r = control.msg(json!({"t": "mode.pinned", "id": 5, "handle": handle}));
    assert_eq!(r["mode"], "simple", "{r}");
    assert_eq!(r["role"], "control");
    assert_eq!(r["locked"], false);
    let r = admin.msg(json!({"t": "mode.pinned", "id": 6}));
    assert!(r["mode"].is_null(), "pinned per role: {r}");
}

#[test]
fn wp_p1hl_a_slot_load_past_the_profile_is_read_back_held_and_reported() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-F");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    // Slot 1 holds output 1 at +10 dB, saved before any profile.
    assert_eq!(admin.set_gain(0, 10.0).status, status::OK);
    let saved = admin.command(Dir::Get, op::REQ_PRESET_SAVE, 1, 1, Vec::new());
    assert_eq!(saved.status, status::OK);
    assert_eq!(admin.set_gain(0, 0.0).status, status::OK);
    assert!(ok(&admin.set_profile(gain_profile())));
    let loaded = control.command(Dir::Get, op::REQ_PRESET_LOAD, 1, 1, Vec::new());
    assert_eq!(loaded.status, status::OK);
    let held = control.event("limits.held");
    assert!(
        held["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().starts_with("out.gain[0] 10 → 3")),
        "{held}"
    );
    assert_eq!(admin.gain(0), 3.0);
}

#[test]
fn wp_p1hl_a_narrowed_profile_holds_the_device_at_once() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-G");
    let admin = Client::open(&hub, handle, Role::Admin);
    assert_eq!(admin.set_gain(0, 6.0).status, status::OK);
    assert_eq!(
        admin.set_f32(op::REQ_SET_MASTER_VOLUME, 0, -3.0).status,
        status::OK
    );
    let set = admin.set_profile(gain_profile());
    assert!(ok(&set), "{set}");
    let held: Vec<&str> = set["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert!(
        held.iter().any(|r| r.starts_with("out.gain[0] 6 → 3")),
        "{held:?}"
    );
    assert!(
        held.iter().any(|r| r.starts_with("vol.master -3 → -10")),
        "{held:?}"
    );
    assert_eq!(admin.gain(0), 3.0);
    assert_eq!(admin.get_f32(op::REQ_GET_MASTER_VOLUME, 0), -10.0);
}

#[test]
fn wp_p1hl_the_hubs_revert_passes_a_limit_set_during_its_countdown() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-H");
    let admin = Client::open(&hub, handle, Role::Admin);
    assert_eq!(admin.set_gain(0, 6.0).status, status::OK);
    let begun = apply_guard::handle_message(
        &admin.hub,
        &admin.session,
        &json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": 1500}),
    )
    .unwrap();
    assert!(begun.contains("\"ok\""), "{begun}");
    assert!(ok(&admin.set_profile(gain_profile())));
    assert_eq!(admin.gain(0), 3.0, "held at once");
    let ended = admin.event("apply.ended");
    assert_eq!(ended["result"], "reverted", "{ended}");
    // The revert restored the state captured before the profile, and nothing
    // holds it back afterwards.
    assert_eq!(admin.gain(0), 6.0);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(admin.gain(0), 6.0, "the revert is never held");
}

/// Coordinator, round 2: only the revert's own change is exempt (by origin), so a
/// Control client's slot load right after a revert is still held.
#[test]
fn wp_p1hl_a_slot_load_right_after_a_revert_is_held() {
    let hub = hub();
    let handle = plug(&hub, "P1HL-R");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    // Slot 1 holds output 1 at +10 dB; the device plays +6 dB.
    assert_eq!(admin.set_gain(0, 10.0).status, status::OK);
    let saved = admin.command(Dir::Get, op::REQ_PRESET_SAVE, 1, 1, Vec::new());
    assert_eq!(saved.status, status::OK);
    assert_eq!(admin.set_gain(0, 6.0).status, status::OK);
    let begun = apply_guard::handle_message(
        &admin.hub,
        &admin.session,
        &json!({"t": "apply.begin", "id": 1, "handle": handle, "timeout_ms": 1000}),
    )
    .unwrap();
    assert!(begun.contains("\"ok\""), "{begun}");
    assert!(ok(&admin.set_profile(gain_profile())));
    assert_eq!(admin.gain(0), 3.0, "held at once");
    let ended = control.event("apply.ended");
    assert_eq!(ended["result"], "reverted", "{ended}");
    let reverted_at = Instant::now();
    assert_eq!(admin.gain(0), 6.0, "the revert's own change is not held");
    let loaded = control.command(Dir::Get, op::REQ_PRESET_LOAD, 1, 1, Vec::new());
    assert_eq!(loaded.status, status::OK);
    assert!(reverted_at.elapsed() < Duration::from_secs(1), "within 1 s");
    let held = control.event("limits.held");
    assert!(
        held["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().starts_with("out.gain[0] 10 → 3")),
        "{held}"
    );
    assert_eq!(admin.gain(0), 3.0);
}

// ── Scenes (OD-P1l) ──────────────────────────────────────────────────────────

#[test]
fn wp_p1hl_a_system_scene_above_a_limit_is_refused() {
    let hub = hub();
    hub.attach(
        Box::new(
            FakeDevice::demo_alink_as(AlinkSetup {
                role: AlinkRole::Sender,
                ..AlinkSetup::default()
            })
            .with_serial("P1HL-TX"),
        ),
        0,
    )
    .unwrap();
    let k = plug(&hub, "P1HL-K");
    let model = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "P1HL-TX"}],
        "units": [{"id": "K", "name": "K340", "serial": "P1HL-K", "room": "main", "upstream": "tx"}],
        "scenes": [
            {"id": "loud", "name": "Loud", "actions": [{"target": "unit:K", "op": "master_volume", "value": -500}]},
            {"id": "quiet", "name": "Quiet", "actions": [{"target": "unit:K", "op": "master_volume", "value": -2000}]},
            {"id": "far", "name": "Far", "actions": [{"target": "unit:K", "op": "out_delay", "arg": 0, "value": 480}]}
        ]
    });
    let h = hub.handle();
    h.system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let admin = Client::open(&hub, k, Role::Admin);
    let mut p = gain_profile();
    p["outputs"][0]["delayMaxMs"] = json!(5.0);
    assert!(ok(&admin.set_profile(p)));
    let before = admin.get_f32(op::REQ_GET_MASTER_VOLUME, 0);
    for scene in ["loud", "far"] {
        let e = h.system_apply_scene(scene, Role::Admin).unwrap_err();
        let text = e.to_string();
        assert!(text.contains("limit_exceeded"), "{text}");
        assert!(text.contains("P1HL-K"), "{text}");
    }
    assert_eq!(
        admin.get_f32(op::REQ_GET_MASTER_VOLUME, 0),
        before,
        "nothing was sent"
    );
    let report = h.system_apply_scene("quiet", Role::Admin).unwrap();
    let k_write = report.writes.iter().find(|w| w.serial == "P1HL-K").unwrap();
    assert!(
        k_write.result == "applied_now" || k_write.result == "queued",
        "{k_write:?}"
    );
}

// ── The store ────────────────────────────────────────────────────────────────

fn store_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let d = std::env::temp_dir().join("wp-p1hl").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A hub store as the hub keeps limits in it.
struct FileStore(Mutex<HubStore>);

impl LimitsPersist for FileStore {
    fn load(&self) -> StoredLimits {
        self.0.lock().unwrap().limits()
    }
    fn save(&self, data: &StoredLimits) -> Result<(), HubError> {
        let mut s = self.0.lock().unwrap();
        s.set_limits(data);
        s.save()
    }
}

#[test]
fn wp_p1hl_the_store_keeps_no_plain_pin_and_a_bundle_no_hash() {
    let d = store_dir("store");
    let persist = Arc::new(FileStore(Mutex::new(HubStore::load(&d).unwrap())));
    let l = HubLimits::new(Some(persist as Arc<dyn LimitsPersist>));
    const PIN: &str = "pin-Kx91-wp-p1hl";
    l.set_pin("SER-1", Some(PIN)).unwrap();
    l.pin_mode(Role::Control, Some("simple")).unwrap();
    let text = std::fs::read_to_string(d.join("hub-store.json")).unwrap();
    assert!(!text.contains(PIN), "a plain PIN on disk");
    let v: Value = serde_json::from_str(&text).unwrap();
    let rec = &v["device_pins"]["SER-1"];
    let (hash, salt) = (rec["hash"].as_str().unwrap(), rec["salt"].as_str().unwrap());
    assert_eq!(hash.len(), 64);
    assert!(!salt.is_empty());
    // Read back by a new hub: the PIN still unlocks, a wrong one does not.
    let again = HubLimits::new(Some(
        Arc::new(FileStore(Mutex::new(HubStore::load(&d).unwrap()))) as Arc<dyn LimitsPersist>,
    ));
    assert!(again.pin_set("SER-1"));
    assert!(
        again
            .unlock("SER-1", 7, "wrong-one", Instant::now())
            .is_err()
    );
    assert!(again.unlock("SER-1", 7, PIN, Instant::now()).is_ok());
    assert_eq!(again.pinned_mode(Role::Control).as_deref(), Some("simple"));
    // The diagnostics bundle carries the store without the hash or the salt.
    let core = dspi_core::DspiCore::new();
    let zip = core
        .diagnostics_zip(dspi_core::DiagOptions {
            app: "wp_p1hl".into(),
            hub_store_dir: d.to_string_lossy().into_owned(),
            files: Vec::new(),
        })
        .unwrap();
    for secret in [hash, salt, PIN] {
        assert!(
            !zip.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{secret} is in the bundle"
        );
    }
    assert!(
        zip.windows(b"hub-store.json".len())
            .any(|w| w == b"hub-store.json"),
        "the store is in the bundle"
    );
}

// ── Over the socket ─────────────────────────────────────────────────────────

/// The server's wiring: `hello` lists the cap, the messages are answered on a
/// real connection, and the store keeps the profile across a restart.
#[test]
fn wp_p1hl_the_messages_go_over_the_socket() {
    use std::net::{IpAddr, Ipv4Addr, TcpStream};
    use tungstenite::Message;

    let dir = store_dir("socket");
    let start = |dir: &PathBuf| {
        Hub::start(HubConfig {
            network: Some(dspi_hub::NetworkConfig {
                port: 0,
                access: dspi_hub::AccessMode::Open,
                bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
                ..dspi_hub::NetworkConfig::new(dir.clone())
            }),
            initial_scan: false,
            ..HubConfig::in_process("P1HL hub")
        })
        .unwrap()
    };
    let hub = start(&dir);
    let handle = plug(&hub, "P1HL-W");
    let port = hub.network().unwrap().port();
    let url = format!("ws://127.0.0.1:{port}/dspi/v1");
    let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    let (mut ws, _) = tungstenite::client(req, tcp).unwrap();
    let next = |ws: &mut tungstenite::WebSocket<TcpStream>, t: &str| -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            if let Message::Text(text) = ws.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["t"] == t {
                    return v;
                }
            }
        }
    };
    ws.send(Message::text(
        r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"p1hl","app":"test","version":"0"}}"#,
    ))
    .unwrap();
    let hello = next(&mut ws, "hello");
    assert!(
        hello["caps"].as_array().unwrap().contains(&json!("limits")),
        "{hello}"
    );
    let set = json!({"t": "limits.set", "id": 1, "handle": handle, "profile": gain_profile()});
    ws.send(Message::text(set.to_string())).unwrap();
    let answer = next(&mut ws, "ok");
    assert_eq!(answer["id"], 1, "{answer}");
    ws.send(Message::text(
        json!({"t": "limits.get", "id": 2, "handle": handle}).to_string(),
    ))
    .unwrap();
    let got = next(&mut ws, "ok");
    assert_eq!(got["profile"]["masterVolumeMaxDb"], -10.0, "{got}");
    ws.send(Message::text(
        json!({"t": "mode.pinned", "id": 3}).to_string(),
    ))
    .unwrap();
    let mode = next(&mut ws, "ok");
    assert_eq!(mode["role"], "admin", "open access is Admin: {mode}");
    drop(ws);
    hub.stop();
    drop(hub);
    // A new hub on the same store has the profile, and judges by it.
    let hub = start(&dir);
    let handle = plug(&hub, "P1HL-W");
    let control = Client::open(&hub, handle, Role::Control);
    assert_eq!(
        refusal(&control.set_gain(0, 6.0))["refused"],
        "limit_exceeded"
    );
}

// ── WP P1DKFIX: review p1d-core's probes, adopted as tests of the fixes ─────

/// A locked device's Control client: write-as-read commands that change the
/// device (preset save/delete, flash save, names, images, a reset) are outside
/// the lock's scope; the analyser (F7, decided) is inside it, as in the core.
#[test]
fn wp_p1dkfix_locked_control_write_as_reads_are_refused_the_analyser_runs() {
    let hub = hub();
    let handle = plug(&hub, "DK-LOCK");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq"]);
    assert!(ok(&admin.set_profile(p)));
    assert!(ok(&admin.msg(
        json!({"t": "limits.pin", "id": 3, "handle": handle, "pin": "2468"})
    )));
    assert_eq!(control.get()["locked"], true);
    type Try = (&'static str, Dir, u8, u16, u16, Vec<u8>);
    let tries: Vec<Try> = vec![
        (
            "PRESET_SAVE",
            Dir::Get,
            op::REQ_PRESET_SAVE,
            1,
            1,
            Vec::new(),
        ),
        (
            "PRESET_DELETE",
            Dir::Get,
            op::REQ_PRESET_DELETE,
            1,
            1,
            Vec::new(),
        ),
        (
            "SAVE_PARAMS",
            Dir::Get,
            op::REQ_SAVE_PARAMS,
            0,
            1,
            Vec::new(),
        ),
        (
            "SET_CHANNEL_NAME",
            Dir::Set,
            op::REQ_SET_CHANNEL_NAME,
            0,
            4,
            b"Left".to_vec(),
        ),
        (
            "SET_EQ_PARAM",
            Dir::Set,
            op::REQ_SET_EQ_PARAM,
            0,
            16,
            vec![0u8; 16],
        ),
        (
            "SET_ALL_PARAMS_CHUNK",
            Dir::Set,
            op::REQ_SET_ALL_PARAMS_CHUNK,
            0,
            4,
            vec![0u8; 4],
        ),
        (
            "FACTORY_RESET",
            Dir::Get,
            op::REQ_FACTORY_RESET,
            0,
            1,
            Vec::new(),
        ),
    ];
    for (what, dir, code, wv, len, payload) in tries {
        let r = control.command(dir, code, wv, len, payload);
        assert_eq!(r.status, status::DENIED, "{what}: {r:?}");
    }
    // The analyser: start and configure pass the lock.
    for (what, dir, code, wv, len, payload) in [
        (
            "RTA_CONTROL",
            Dir::Get,
            op::REQ_RTA_CONTROL,
            1,
            1,
            Vec::new(),
        ),
        (
            "RTA_SET_CONFIG",
            Dir::Set,
            op::REQ_RTA_SET_CONFIG,
            0,
            4,
            vec![0, 0, 0, 0],
        ),
    ] {
        let r = control.command(dir, code, wv, len, payload);
        assert_ne!(r.status, status::DENIED, "{what}: {r:?}");
    }
    // Volume, mute, preset load pass.
    assert_eq!(
        control.set_f32(op::REQ_SET_USER_VOLUME, 0, -12.0).status,
        status::OK
    );
    assert_eq!(
        control
            .command(Dir::Get, op::REQ_PRESET_LOAD, 1, 1, Vec::new())
            .status,
        status::OK
    );
}

/// F1: a Control client cannot write output 0's gain past the limit by naming
/// it output 0x0100 (the firmware reads `wValue & 0xFF`).
#[test]
fn wp_p1dkfix_a_wide_output_index_is_judged_over_link() {
    let hub = hub();
    let handle = plug(&hub, "DK-WIDE");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    assert!(ok(&admin.set_profile(gain_profile())));
    let before = control.gain(0);
    assert_eq!(
        refusal(&control.set_gain(0, 10.0))["refused"],
        "limit_exceeded"
    );
    for wide in [0x0100u16, 0xFF00] {
        let r = control.set_gain(wide, 10.0);
        assert_eq!(
            refusal(&r)["refused"],
            "limit_exceeded",
            "{wide:#06x}: {r:?}"
        );
        assert_eq!(refusal(&r)["limit"], "out_gain");
    }
    assert_eq!(control.gain(0), before, "the device was not written");
    // Unreadable: refused too, and an Admin's alike.
    let short = control.command(Dir::Set, op::REQ_SET_OUTPUT_GAIN, 0, 2, vec![0, 0]);
    assert_eq!(refusal(&short)["refused"], "limit_exceeded");
    let nan = admin.set_gain(0, f32::NAN);
    assert_eq!(refusal(&nan)["refused"], "limit_exceeded");
}

/// F6 and the client's prompt: `limits.get` says the wrong tries and the
/// window left, and the store keeps them across a restart.
#[test]
fn wp_p1dkfix_limits_get_reports_the_window_and_the_store_keeps_it() {
    let d = store_dir("tries");
    let persist = Arc::new(FileStore(Mutex::new(HubStore::load(&d).unwrap())));
    let l = HubLimits::new(Some(persist as Arc<dyn LimitsPersist>));
    l.set_pin("SER-T", Some("2468")).unwrap();
    let now = Instant::now();
    for _ in 0..=dspi_hub::limits::LOCK_FREE_TRIES {
        let _ = l.unlock("SER-T", 3, "0000", now);
    }
    let text = std::fs::read_to_string(d.join("hub-store.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["device_pin_tries"]["SER-T"]["wrong"],
        dspi_hub::limits::LOCK_FREE_TRIES + 1,
        "{text}"
    );
    let again = HubLimits::new(Some(
        Arc::new(FileStore(Mutex::new(HubStore::load(&d).unwrap()))) as Arc<dyn LimitsPersist>,
    ));
    assert!(again.tries("SER-T", Instant::now()).1.is_some());
    // Over a hub: the answer carries the count and the window.
    let hub = hub();
    let handle = plug(&hub, "DK-GET");
    let admin = Client::open(&hub, handle, Role::Admin);
    let control = Client::open(&hub, handle, Role::Control);
    let mut p = gain_profile();
    p["lockedPages"] = json!(["eq"]);
    assert!(ok(&admin.set_profile(p)));
    assert!(ok(&admin.msg(
        json!({"t": "limits.pin", "id": 3, "handle": handle, "pin": "2468"})
    )));
    for i in 0..=dspi_hub::limits::LOCK_FREE_TRIES {
        let _ = control
            .msg(json!({"t": "limits.unlock", "id": 10 + i, "handle": handle, "pin": "0000"}));
    }
    let st = control.get();
    assert_eq!(
        st["wrong_tries"],
        dspi_hub::limits::LOCK_FREE_TRIES + 1,
        "{st}"
    );
    assert!(st["retry_in_ms"].as_u64().unwrap() > 0, "{st}");
    let later =
        control.msg(json!({"t": "limits.unlock", "id": 30, "handle": handle, "pin": "2468"}));
    assert_eq!(later["code"], "retry_later", "{later}");
}
