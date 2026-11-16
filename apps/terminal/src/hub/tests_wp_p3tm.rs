//! WP P3TM's done criteria: `uconsole hub`'s groups, firmware and rollout commands
//! against an in-process hub with the demo device, one more device "on its USB" (a demo
//! behind a wrapper that notes its bootloader entry), and a fake BOOTSEL scan and writer
//! injected through the hub's configuration. No USB, no real device.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{BootDevice, FirmwareGuardBoard, FlashReport};
use dspi_hub::rollout::{BootScan, Writer};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, Transport,
    TransportCaps,
};
use dspi_tui::shell::connect::{LinkTarget, Target};
use serde_json::{Value, json};

use super::super::{
    Failure, HubCommand, RunOptions, Running, Services, TlsMode, parse, run, start_with,
};
use super::*;
use crate::api::status;
use crate::api::tests_wp_p2tm::api_call;
use crate::exit;

const WAIT: Duration = Duration::from_secs(30);
/// The DSPi's USB vendor id: a device "on this hub's USB".
const VID: u16 = 0x2E8B;
const DEMO: &str = "DEMO0000DSPI0001";
const DEV: &str = "P3TM0000DEV00001";
const IMAGE: &str = "dspi-1.1.7.uf2";

fn scratch(name: &str) -> PathBuf {
    // Beside the test binary, in the target directory: never on tmpfs.
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("wp_p3tm")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

use std::path::PathBuf;

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn parsed(args: &[&str]) -> Result<HubCommand, Failure> {
    parse(args, true, Some(PathBuf::from("/store")))
}

fn run_options(args: &[&str]) -> RunOptions {
    match parsed(args) {
        Ok(HubCommand::Run(o)) => o,
        other => panic!("{args:?}: {other:?}"),
    }
}

// ── Images (laid out as the firmware guard's own tests build them) ──────────

const XIP: u32 = 0x1000_0000;
const RP2350: u32 = 0xE48B_FF59;
const BI_MARKER_START: u32 = 0x7188_EBF2;
const BI_MARKER_END: u32 = 0xE71A_A390;
const BI_TYPE_ID_AND_STRING: u16 = 6;
const TAG_RP: u16 = 0x5052;
const TAG_DS: u16 = 0x5344;
const ID_NAME: u32 = 0x0203_1C86;
const ID_VERSION: u32 = 0x11A9_BC3A;
const ID_STAMP: u32 = 0x0B7A_4C15;

fn block(target: u32, page: &[u8], no: u32, total: u32) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    for (off, v) in [
        (0, 0x0A32_4655u32),
        (4, 0x9E5D_5157),
        (8, 0x2000),
        (12, target),
        (16, 256),
        (20, no),
        (24, total),
        (28, RP2350),
        (508, 0x0AB1_6F30),
    ] {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[32..32 + page.len()].copy_from_slice(page);
    b
}

/// An RP2350 DSPi image of `version`, stamped `stamp` mV.
fn image(version: &str, stamp: &str) -> Vec<u8> {
    let entries: Vec<(u16, u32, String)> = vec![
        (TAG_RP, ID_NAME, "DSPi".into()),
        (TAG_RP, ID_VERSION, version.into()),
        (TAG_DS, ID_STAMP, stamp.into()),
    ];
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, BI_MARKER_START);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, BI_MARKER_END);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&BI_TYPE_ID_AND_STRING.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        out.extend(block(XIP + 256 * i as u32, p, i as u32, pages.len() as u32));
    }
    out
}

/// `IMAGE` written to a file of that name in `dir`.
fn image_file(dir: &Path) -> PathBuf {
    let path = dir.join(IMAGE);
    std::fs::write(&path, image("1.1.7", "1200")).unwrap();
    path
}

// ── The device "on USB", the fake BOOTSEL scan and writer ───────────────────

/// A demo device that takes `REQ_ENTER_BOOTLOADER` (and notes it).
struct Dev {
    inner: FakeDevice,
    entered: Arc<AtomicBool>,
}

impl Transport for Dev {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.entered.store(true, Ordering::SeqCst);
            return Ok(vec![0]);
        }
        self.inner.control_in(opcode, value, len)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.entered.store(true, Ordering::SeqCst);
            return Ok(vec![0]);
        }
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(opcode, value, data)
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
}

/// The demo device as firmware `1.1.<patch>` with a 1.20 V core default (a K340).
fn demo_as(patch: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    // `GET_PLATFORM`: major, (minor << 4) | patch, outputs, minor, patch, beta.
    capture.platform[2] = 0x10 | (patch & 0x0F);
    capture.platform[5] = patch;
    let d = FakeDevice::new(
        capture,
        FakeOptions {
            extensions: true,
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(DEV);
    d.remote().vreg_set_build_default(0x0D);
    d
}

/// One device on the "USB", its boot ROM, and the writer.
#[derive(Default)]
struct Bench {
    hub: Mutex<Option<HubHandle>>,
    entered: Mutex<Option<Arc<AtomicBool>>>,
    flashes: Mutex<Vec<usize>>,
}

impl Bench {
    fn plug(&self, patch: u8) {
        let entered = Arc::new(AtomicBool::new(false));
        *self.entered.lock().unwrap() = Some(Arc::clone(&entered));
        let hub = self.hub.lock().unwrap().clone().unwrap();
        hub.attach(
            Box::new(Dev {
                inner: demo_as(patch),
                entered,
            }),
            VID,
        )
        .unwrap();
    }

    fn in_boot(&self) -> bool {
        self.entered
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|e| e.load(Ordering::SeqCst))
    }
}

impl BootScan for Bench {
    fn boards(&self) -> Vec<BootDevice> {
        if !self.in_boot() {
            return Vec::new();
        }
        // The device left the bus for its boot ROM.
        let hub = self.hub.lock().unwrap().clone().unwrap();
        if let Some(d) = hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == DEV && d.state == dspi_hub::DeviceState::Online)
        {
            let _ = hub.detach(d.handle);
        }
        vec![BootDevice {
            platform: 1,
            serial: Some(DEV.into()),
            bus_location: "1-2".into(),
        }]
    }
}

impl Writer for Bench {
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        assert_eq!(board.required_mv, 1200, "the guard's board record");
        self.flashes.lock().unwrap().push(image.len());
        // Back with the image's firmware.
        self.plug(7);
        Ok(FlashReport {
            platform: target.platform,
            serial: target.serial.clone(),
            firmware_version: "1.1.7".into(),
            sectors: 2,
            bytes_written: 8192,
            elapsed_ms: 5,
        })
    }
}

/// A demo hub over TLS (`on`) with the rollout and the bench's scan and writer.
fn rig(name: &str, extra: &[&str]) -> (Running, Arc<Bench>, PathBuf) {
    let dir = scratch(name);
    let mut args = vec![
        "--port",
        "0",
        "--loopback",
        "--no-keep-awake",
        "--tls",
        "on",
        "--rollout",
    ];
    args.extend_from_slice(extra);
    let mut o = run_options(&args);
    o.name = Some("P3TM test hub".into());
    o.store = dir.join("store");
    let bench = Arc::new(Bench::default());
    let b = Arc::clone(&bench);
    let running = start_with(&o, false, move |config| {
        let r = config.services.rollout.as_mut().expect("--rollout");
        r.boot_scan = Some(Arc::clone(&b) as Arc<dyn BootScan>);
        r.writer = Some(b as Arc<dyn Writer>);
    })
    .unwrap();
    *bench.hub.lock().unwrap() = Some(running.handle.clone());
    (running, bench, dir)
}

fn say(running: &Running, line: &str) -> Vec<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    prompt(running, &words, false).unwrap_or_else(|| panic!("`{line}` is not a P3TM command"))
}

fn say_json(running: &Running, line: &str) -> Value {
    let words: Vec<&str> = line.split_whitespace().collect();
    let lines = prompt(running, &words, true).unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    serde_json::from_str(&lines[0]).unwrap()
}

fn joined(lines: &[String]) -> String {
    lines.join("\n")
}

fn in_inventory(running: &Running, serial: &str) -> bool {
    ask(running, Role::Viewer, "inventory.list", json!({})).is_ok_and(|v| {
        v["devices"]
            .as_array()
            .is_some_and(|d| d.iter().any(|d| d["serial"] == serial))
    })
}

// ── The switches ────────────────────────────────────────────────────────────

#[test]
fn wp_p3tm_groups_are_on_and_the_rollout_is_opt_in() {
    let o = run_options(&[]);
    assert!(o.services.groups, "groups on by default");
    assert!(
        !o.rollout && !o.allow_unverified_firmware,
        "the rollout off"
    );
    let mut config = dspi_hub::ServiceConfig::default();
    o.services.apply(&mut config);
    assert!(config.groups);

    let o = run_options(&["--no-groups"]);
    assert_eq!(
        o.services,
        Services {
            groups: false,
            ..Services::default()
        }
    );
    let mut config = dspi_hub::ServiceConfig::default();
    o.services.apply(&mut config);
    assert!(!config.groups);

    // TLS required by default on a network address: the rollout can run.
    let o = run_options(&["--rollout"]);
    assert!(o.rollout && !o.allow_unverified_firmware);
    let o = run_options(&["--rollout", "--allow-unverified-firmware"]);
    assert!(o.rollout && o.allow_unverified_firmware);
    assert!(run_options(&["--loopback", "--tls", "on", "--rollout"]).rollout);
    assert!(run_options(&["--upstream-compat", "--rollout"]).rollout);

    for bad in [
        // The bench switch is the rollout's.
        &["--allow-unverified-firmware"][..],
        // No TLS, no rollout: --tls off, and loopback's default.
        &["--rollout", "--tls", "off"],
        &["--loopback", "--rollout"],
        // Each is for running a hub.
        &["clients", "--rollout"],
        &["clients", "--no-groups"],
        &["api-key", "list", "--allow-unverified-firmware"],
    ] {
        assert!(
            matches!(parse(bad, false, Some("/s".into())), Err(Failure::Usage(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn wp_p3tm_firmware_commands_parse() {
    let p = |args: &[&str]| parse(args, false, Some(PathBuf::from("/store")));
    assert_eq!(
        p(&["firmware", "add", "x.uf2"]).unwrap(),
        HubCommand::FirmwareAdd {
            store: "/store".into(),
            data: None,
            file: "x.uf2".into(),
        }
    );
    assert_eq!(
        p(&["firmware", "add", "x.uf2", "--data", "/d", "--store", "/s"]).unwrap(),
        HubCommand::FirmwareAdd {
            store: "/s".into(),
            data: Some("/d".into()),
            file: "x.uf2".into(),
        }
    );
    assert_eq!(
        p(&["firmware", "list"]).unwrap(),
        HubCommand::FirmwareList {
            store: "/store".into(),
            data: None,
        }
    );
    for bad in [
        &["firmware"][..],
        &["firmware", "add"],
        &["firmware", "add", "a.uf2", "b.uf2"],
        &["firmware", "frob"],
        &["firmware", "list", "--rollout"],
        // `--data` stays a running hub's for every other command.
        &["clients", "--data", "/d"],
    ] {
        assert!(matches!(p(bad), Err(Failure::Usage(_))), "{bad:?}");
    }
    assert!(parsed(&["firmware", "list"]).is_err(), "--demo is a hub's");
}

// ── The prompt ──────────────────────────────────────────────────────────────

/// The entry's done criteria at the prompt: a group set and listed, an image added and
/// listed with its label, a rollout started from the prompt and its result printed.
#[test]
fn wp_p3tm_the_prompt_sets_groups_adds_an_image_and_rolls_out() {
    let (running, bench, dir) = rig("prompt", &["--allow-unverified-firmware"]);
    bench.plug(6);
    wait_until("the inventory has both devices", || {
        in_inventory(&running, DEV) && in_inventory(&running, DEMO)
    });

    // Groups.
    assert_eq!(say(&running, "groups"), ["no groups"]);
    let set = joined(&say(&running, &format!("group set Bar {DEV}")));
    assert!(
        set.starts_with("group created: g1  Bar  (1 member)"),
        "{set}"
    );
    let listed = joined(&say(&running, "groups"));
    assert_eq!(listed, format!("g1  Bar  (1 member): {DEV}"));
    let v = say_json(&running, &format!("group set g1 Bar {DEV} {DEMO}"));
    assert_eq!(v["event"], "group");
    assert_eq!(v["created"], false);
    assert_eq!(v["group"]["members"], json!([DEV, DEMO]));
    let v = say_json(&running, "groups");
    assert_eq!(v["groups"][0]["id"], "g1");
    // A refusal is the service's: a member the inventory does not know.
    let nope = joined(&say(&running, "group set Other NOPE0000"));
    assert!(nope.contains("(not_found)"), "{nope}");
    assert!(say(&running, "group frob")[0].contains("group set"));
    let v = say_json(&running, &format!("group set g1 Bar {DEV}"));
    assert_eq!(v["group"]["members"], json!([DEV]));

    // An image, by its file, as `Hub console`, labelled not verified.
    assert_eq!(
        say(&running, "firmware list"),
        ["no firmware images stored"]
    );
    let file = image_file(&dir);
    let added = joined(&say(&running, &format!("firmware add {}", file.display())));
    assert!(
        added.starts_with(&format!(
            "added {IMAGE}  NOT VERIFIED  RP2350  fw 1.1.7  1.20 V"
        )),
        "{added}"
    );
    assert!(added.contains("added by Hub console"), "{added}");
    let again = joined(&say(&running, &format!("firmware add {}", file.display())));
    assert!(again.starts_with("already stored:"), "{again}");
    let listed = joined(&say(&running, "firmware list"));
    assert!(
        listed.starts_with(&format!("{IMAGE}  NOT VERIFIED")),
        "{listed}"
    );
    let v = say_json(&running, "firmware list");
    assert_eq!(v["images"][0]["verified"], false);
    assert_eq!(v["images"][0]["added_by"], "Hub console");
    // The store's checks: not an image; a name the store does not take.
    let junk = dir.join("junk.uf2");
    std::fs::write(&junk, b"not a UF2").unwrap();
    let refused = joined(&say(&running, &format!("firmware add {}", junk.display())));
    assert!(refused.contains("(bad_request)"), "{refused}");
    let bad_name = dir.join("image.bin");
    std::fs::write(&bad_name, image("1.1.7", "1200")).unwrap();
    let refused = joined(&say(
        &running,
        &format!("firmware add {}", bad_name.display()),
    ));
    assert!(refused.contains("ending in .uf2"), "{refused}");

    // Unmuted, without --while-live: refused, and the prompt says what to add.
    let refused = joined(&say(&running, &format!("rollout {DEV} {IMAGE}")));
    assert!(
        refused.contains("(confirmation_required)") && refused.contains("--while-live"),
        "{refused}"
    );
    // The demo device is not on this hub's USB.
    let refused = joined(&say(
        &running,
        &format!("rollout {DEMO} {IMAGE} --while-live"),
    ));
    assert!(refused.contains("(unsupported)"), "{refused}");
    assert!(bench.flashes.lock().unwrap().is_empty(), "nothing flashed");
    assert!(!bench.in_boot());

    // The group's rollout, from the prompt.
    let started = joined(&say(
        &running,
        &format!("rollout group:g1 {IMAGE} --while-live"),
    ));
    assert!(started.starts_with("rollout ro1 started"), "{started}");
    assert!(
        started.contains("ro1  running  group:g1  image dspi-1.1.7.uf2 (fw 1.1.7)  by Hub console"),
        "{started}"
    );
    let mut ends = Vec::new();
    wait_until("the rollout's end is printed", || {
        ends.extend(rollout_ends(&running, false));
        !ends.is_empty()
    });
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert!(
        ends[0].starts_with(&format!("rollout: {DEV} updated to firmware 1.1.7: ")),
        "{ends:?}"
    );
    assert_eq!(bench.flashes.lock().unwrap().len(), 1);
    wait_until("the record ends", || {
        say(&running, "rollout status ro1")[0].starts_with("ro1  ok")
    });
    let status = say(&running, "rollout status");
    assert!(status[0].starts_with("ro1  ok  group:g1"), "{status:?}");
    assert!(
        status[1].starts_with(&format!("    {DEV}  ok  at verify: ")),
        "{status:?}"
    );
    assert!(status[1].contains("; settings "), "{status:?}");
    let v = say_json(&running, "rollout status ro1");
    assert_eq!(v["rollouts"][0]["by"], "Hub console");
    assert_eq!(v["rollouts"][0]["devices"][0]["after"]["fw"], "1.1.7");
    // A finished rollout is not cancelled; an unknown one is not found.
    assert!(joined(&say(&running, "rollout cancel ro1")).contains("(conflict)"));
    assert!(joined(&say(&running, "rollout status ro9")).contains("(not_found)"));
    assert!(say(&running, "rollout")[0].contains("rollout <serial|group:id>"));

    // The group goes, by id.
    assert_eq!(say(&running, "group delete g1"), ["group g1 deleted"]);
    assert_eq!(say(&running, "groups"), ["no groups"]);
    assert!(
        prompt(&running, &["devices"], false).is_none(),
        "not P3TM's"
    );
    drop(running);
}

/// Groups off, and a hub without `--rollout`: each command says so.
#[test]
fn wp_p3tm_the_prompt_says_when_a_family_is_off() {
    let dir = scratch("off");
    let mut o = run_options(&[
        "--port",
        "0",
        "--loopback",
        "--no-keep-awake",
        "--no-groups",
    ]);
    o.store = dir.join("store");
    let running = start_with(&o, false, |_| {}).unwrap();
    assert!(running.rollout_ends.is_none());
    assert_eq!(
        say(&running, "groups"),
        ["groups are off on this hub (--no-groups)"]
    );
    assert_eq!(
        say(&running, "firmware list"),
        ["the firmware rollout is off on this hub: start it with --rollout"]
    );
    let v = say_json(&running, "rollout status");
    assert_eq!(v["event"], "error");
    assert_eq!(v["code"], "unknown_type");
    drop(running);
}

// ── TLS only, and the exit codes ────────────────────────────────────────────

/// The prompt's session counts as encrypted (nothing travels); any other principal the
/// hub opens does not, and the rollout refuses it `tls_required`.
#[test]
fn wp_p3tm_only_the_prompt_session_is_taken_as_encrypted() {
    let (running, _bench, _dir) = rig("encrypted", &[]);
    let console = running
        .handle
        .open_console_principal(Role::Admin, "Hub console")
        .unwrap();
    assert!(console.encrypted());
    running.handle.close_session(console.id());
    let plain = running
        .handle
        .open_principal(Role::Admin, None, "A script")
        .unwrap();
    assert!(!plain.encrypted());
    let reply = running
        .handle
        .service_request(&plain, &json!({"t": "fw.image.list", "id": 1}))
        .unwrap();
    let v: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(v["code"], "tls_required", "{v}");
    running.handle.close_session(plain.id());
    assert!(ask(&running, Role::Viewer, "fw.image.list", json!({})).is_ok());
    drop(running);
}

fn link(running: &Running, dir: &Path) -> Target {
    let pin = running
        .control
        .auth()
        .begin_pairing(Instant::now())
        .unwrap()
        .pin;
    let mut l = LinkTarget::parse(&format!("127.0.0.1:{}", running.control.port())).unwrap();
    l.pin = Some(pin);
    l.tokens = Some(dir.join("link-tokens.toml"));
    Target::Link(l)
}

/// `uconsole api` on the rollout: `denied` (5) for a console that is not an admin, over
/// TLS; `tls_required` (3, a refusal) over plaintext.
#[test]
fn wp_p3tm_api_exit_codes_for_denied_and_tls_required() {
    let (running, _bench, dir) = rig("exit", &[]);
    let target = link(&running, &dir);
    // Paired with the PIN over TLS: control. The reads are anyone's.
    assert_eq!(api_call(&target, "fw.image.list", json!({})), exit::OK);
    assert_eq!(api_call(&target, "fw.rollout.status", json!({})), exit::OK);
    assert_eq!(
        api_call(
            &target,
            "fw.rollout.start",
            json!({"serial": DEMO, "image": IMAGE})
        ),
        status::DENIED
    );
    assert_eq!(
        api_call(&target, "fw.image.delete", json!({"image": IMAGE})),
        status::DENIED
    );
    drop(running);

    // A plaintext session (a hub that serves no TLS; `uconsole hub` refuses --rollout
    // there, so the rollout is put in by hand): every `fw.*` is `tls_required`.
    let dir = scratch("plain");
    let mut o = run_options(&["--port", "0", "--loopback", "--no-keep-awake"]);
    assert_eq!(o.tls.policy(o.loopback).mode, TlsMode::Off);
    o.store = dir.join("store");
    let running = start_with(&o, false, |config| {
        config.services.rollout = Some(dspi_hub::rollout::RolloutConfig {
            allow_unverified_firmware: false,
            boot_scan: None,
            writer: None,
            serves_tls: false,
        });
    })
    .unwrap();
    let target = link(&running, &dir);
    assert_eq!(api_call(&target, "inventory.list", json!({})), exit::OK);
    assert_eq!(
        api_call(&target, "fw.image.list", json!({})),
        exit::REJECTED
    );
    assert_eq!(crate::api::exit_for("tls_required"), exit::REJECTED);
    drop(running);
}

// ── With the hub stopped ────────────────────────────────────────────────────

#[test]
fn wp_p3tm_firmware_add_and_list_with_the_hub_stopped() {
    let dir = scratch("offline");
    let store = dir.join("store");
    let data = dir.join("data");
    let file = image_file(&dir);
    let s = store.to_str().unwrap();
    let d = data.to_str().unwrap();
    let f = file.to_str().unwrap();
    let usb = Target::Usb { serial: None };
    assert_eq!(
        run(
            &usb,
            &["hub", "firmware", "add", f, "--store", s, "--data", d],
            false
        ),
        exit::OK
    );
    let info = data.join("firmware").join("index.json");
    let index: Value = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    assert_eq!(index["images"][0]["name"], IMAGE);
    assert_eq!(index["images"][0]["added_by"], "Hub console");
    assert_eq!(index["images"][0]["verified"], false);
    // The same bytes again: kept, not an error.
    assert_eq!(
        run(
            &usb,
            &["hub", "firmware", "add", f, "--store", s, "--data", d],
            false
        ),
        exit::OK
    );
    assert_eq!(
        run(
            &usb,
            &["hub", "firmware", "list", "--store", s, "--data", d],
            true
        ),
        exit::OK
    );
    // The upload's checks: not a UF2 is refused, nothing stored.
    let junk = dir.join("junk.uf2");
    std::fs::write(&junk, b"nope").unwrap();
    assert_eq!(
        run(
            &usb,
            &[
                "hub",
                "firmware",
                "add",
                junk.to_str().unwrap(),
                "--store",
                s,
                "--data",
                d
            ],
            false
        ),
        exit::USAGE
    );
    let index: Value = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    assert_eq!(index["images"].as_array().unwrap().len(), 1);
    // No `--data`: the store is the data folder, as a running hub's.
    assert_eq!(
        run(&usb, &["hub", "firmware", "add", f, "--store", s], false),
        exit::OK
    );
    assert!(store.join("firmware").join(IMAGE).is_file());

    // While a hub runs on the store, it is the prompt's.
    let mut o = run_options(&["--port", "0", "--loopback", "--no-keep-awake"]);
    o.store = store.clone();
    let running = start_with(&o, false, |_| {}).unwrap();
    assert!(matches!(
        offline_firmware_add(&store, None, &file, false),
        Err(Failure::Usage(m)) if m.contains("a hub is running")
    ));
    drop(running);
    assert!(offline_firmware_list(&store, None, false).is_ok());
}
