//! WP P3CC: the core's typed calls for the hub's fleet families
//! (`dspi_core::hub_server::fleet`, `hub-api.md` sections 19 to 21) against the
//! real hub in-process: groups, group writes, bulk apply, the image store and a
//! rollout driven end to end from the core with a fake BOOTSEL scan and writer.
//! The clients are the real one (`DspiCore` over DSPi Link), over TLS (the hub runs
//! `--tls on` whatever `DSPI_HUB_TEST_TLS` says), and one over plaintext through a
//! proxy that refuses the TLS handshake, for `tls_required`.
//!
//! No USB, no real device: the devices are demo `FakeDevice`s; the one "on the
//! hub's USB" notes its bootloader entry, and "the board in BOOTSEL" is what the
//! fake scan says.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_core::{
    BootDevice, DspiCore, DspiError, FirmwareGuardBoard, FlashReport, HubFleetTarget, HubGroup,
    HubOffset, HubPresetPick, HubServerEvent, ParamValue,
};
use dspi_hub::rollout::{BootScan, RolloutConfig, Writer};
use dspi_hub::{
    AccessMode, HistoryConfig, Hub, HubConfig, HubHandle, NetworkConfig, Role, ServiceConfig,
    TestClock, TlsMode,
};
use dspi_proto::generated::opcodes as op;
use dspi_session::snapshot_store::SnapshotStore;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, Transport,
    TransportCaps,
};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(30);
/// The DSPi's USB vendor id: a device "on this hub's USB".
const VID: u16 = 0x2E8B;
const T0: u64 = 1_800_000_000;
/// Two devices the hub reaches some other way (vid 0), and one on its USB.
const A: &str = "P3CC-A";
const B: &str = "P3CC-B";
const R: &str = "P3CC-R";
const IMAGE: &str = "dspi-1.1.7.uf2";

// ── Logs: no image bytes in them ────────────────────────────────────────────

fn kept() -> &'static Mutex<Vec<String>> {
    static K: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    K.get_or_init(|| Mutex::new(Vec::new()))
}

struct Keep;

impl log::Log for Keep {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        !m.target().starts_with("tungstenite") && !m.target().starts_with("rustls")
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            kept().lock().unwrap().push(format!("{}", r.args()));
        }
    }
    fn flush(&self) {}
}

fn keep_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        static KEEP: Keep = Keep;
        let _ = log::set_logger(&KEEP);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// No run of 100 or more base64 characters in anything logged, the core's
/// lines included: no image bytes.
fn no_base64_in_logs() {
    for line in kept().lock().unwrap().iter() {
        let mut run = 0;
        for c in line.chars() {
            if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' {
                run += 1;
                assert!(run < 100, "a base64 run in the log: {line}");
            } else {
                run = 0;
            }
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p3cc").join(format!(
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

fn link_code(e: &DspiError) -> &str {
    match e {
        DspiError::Link { code, .. } => code,
        _ => "",
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

/// An RP2350 DSPi image of `version`, stamped 1.20 V, padded with `pad` more
/// pages (so the upload takes several chunks).
fn image(version: &str, pad: usize) -> Vec<u8> {
    let entries: Vec<(u16, u32, String)> = vec![
        (TAG_RP, ID_NAME, "DSPi".into()),
        (TAG_RP, ID_VERSION, version.into()),
        (TAG_DS, ID_STAMP, "1200".into()),
    ];
    let mut f = vec![0u8; 0x2000 + 256 * pad];
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
    // Not all zero, so a chunk's base64 is no long run of `A`.
    for (i, b) in f.iter_mut().enumerate().skip(0x2000) {
        *b = (i * 7 % 251) as u8;
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        out.extend(block(XIP + 256 * i as u32, p, i as u32, pages.len() as u32));
    }
    out
}

// ── The device on the hub's USB, and the fake boot ROM ──────────────────────

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

/// The demo device reporting firmware `1.1.<patch>`, core default 1.20 V.
fn demo(serial: &str, patch: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    capture.platform[2] = 0x10 | (patch & 0x0F);
    capture.platform[5] = patch;
    let options = FakeOptions {
        extensions: true,
        surfaces: true,
        analysis: true,
        ..FakeOptions::default()
    };
    let d = FakeDevice::new(capture, options).with_serial(serial);
    d.remote().vreg_set_build_default(0x0D);
    d
}

#[derive(Default)]
struct Bench {
    hub: Mutex<Option<HubHandle>>,
    entered: Mutex<Vec<(String, Arc<AtomicBool>)>>,
    in_boot: Mutex<Option<String>>,
    flashes: Mutex<Vec<(String, usize)>>,
}

impl Bench {
    fn hub(&self) -> HubHandle {
        self.hub.lock().unwrap().clone().unwrap()
    }

    fn plug(&self, serial: &str, patch: u8) -> u8 {
        let entered = Arc::new(AtomicBool::new(false));
        self.entered
            .lock()
            .unwrap()
            .push((serial.to_owned(), Arc::clone(&entered)));
        self.hub()
            .attach(
                Box::new(Dev {
                    inner: demo(serial, patch),
                    entered,
                }),
                VID,
            )
            .unwrap()
    }
}

impl BootScan for Bench {
    fn boards(&self) -> Vec<BootDevice> {
        let entered = self
            .entered
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.load(Ordering::SeqCst))
            .map(|(s, _)| s.clone())
            .next_back();
        let Some(serial) = entered.or_else(|| self.in_boot.lock().unwrap().clone()) else {
            return Vec::new();
        };
        // A device that took its entry leaves the bus, and its boot ROM shows.
        let hub = self.hub();
        if let Some(d) = hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial && d.state == dspi_hub::DeviceState::Online)
        {
            let _ = hub.detach(d.handle);
        }
        *self.in_boot.lock().unwrap() = Some(serial.clone());
        vec![BootDevice {
            platform: 1,
            serial: Some(serial),
            bus_location: "1-2".into(),
        }]
    }
}

impl Writer for Bench {
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        _board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        let serial = self.in_boot.lock().unwrap().take().unwrap_or_default();
        self.flashes
            .lock()
            .unwrap()
            .push((serial.clone(), image.len()));
        for (s, e) in self.entered.lock().unwrap().iter() {
            if *s == serial {
                e.store(false, Ordering::SeqCst);
            }
        }
        // It comes back with the image's firmware.
        self.plug(&serial, 7);
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

// ── The hub and its clients ─────────────────────────────────────────────────

struct Rig {
    hub: Hub,
    bench: Arc<Bench>,
    dir: PathBuf,
    _clock: Arc<TestClock>,
}

/// A hub serving TLS with every fleet family (`every`), or none of them; `A` and
/// `B` attached, and with `every` `R` on its USB.
fn rig(tag: &str, every: bool) -> Rig {
    keep_logs();
    let dir = scratch(tag);
    let clock = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(T0)));
    let bench = Arc::new(Bench::default());
    let services = if every {
        ServiceConfig {
            clock: Arc::clone(&clock) as Arc<dyn dspi_hub::HubClock>,
            inventory: true,
            groups: true,
            alarms: true,
            history: Some(HistoryConfig::default()),
            rollout: Some(RolloutConfig {
                allow_unverified_firmware: true,
                boot_scan: Some(Arc::clone(&bench) as Arc<dyn BootScan>),
                writer: Some(Arc::clone(&bench) as Arc<dyn Writer>),
                serves_tls: true,
            }),
            ..ServiceConfig::default()
        }
    } else {
        ServiceConfig {
            clock: Arc::clone(&clock) as Arc<dyn dspi_hub::HubClock>,
            ..ServiceConfig::default()
        }
    };
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tls: TlsMode::On,
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services,
        ..HubConfig::in_process("p3cc")
    })
    .unwrap();
    *bench.hub.lock().unwrap() = Some(hub.handle());
    for s in [A, B] {
        hub.attach(Box::new(FakeDevice::demo().with_serial(s)), 0)
            .unwrap();
    }
    if every {
        bench.plug(R, 6);
    }
    Rig {
        hub,
        bench,
        dir,
        _clock: clock,
    }
}

impl Rig {
    /// A core with `role` on the hub, over TLS (`via` None) or through `via`'s
    /// port (the plaintext proxy).
    fn core(&self, name: &str, role: Role, via: Option<u16>) -> Arc<DspiCore> {
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
        let port = via.unwrap_or_else(|| net.port());
        let core = DspiCore::new();
        core.link_connect(
            format!("ws://127.0.0.1:{port}/dspi/v1"),
            name.into(),
            "0".into(),
            authed.token.unwrap(),
            String::new(),
        )
        .unwrap();
        assert_eq!(core.link_security().encrypted, via.is_none());
        wait_until("the device list", || core.link_devices().len() >= 2);
        core
    }

    /// `A`'s settings as a snapshot in the hub's store: its id.
    fn snapshot_of_a(&self) -> String {
        let core = DspiCore::new();
        let t = self.hub.handle().open_local(A).unwrap();
        core.open_transport(Box::new(t), A.into(), 0).unwrap();
        let _ = core.feature_present("device_name");
        let text = core
            .snapshot_capture_document("Doors open".into(), String::new())
            .unwrap();
        core.disconnect();
        let store = SnapshotStore::new(self.dir.join("client"));
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        let e = store.save(A, &mut doc, 1_763_200_000_000).unwrap();
        let (_, filed) = store.read(&e.id).unwrap();
        dspi_hub::snapshots::HubSnapshots::open(&self.dir.join("store"))
            .put(A, &filed)
            .unwrap();
        e.id
    }
}

/// A proxy to the hub's port that refuses any TLS handshake (a first byte 0x16)
/// and passes everything else: the core falls back to plaintext through it.
fn plaintext_proxy(hub_port: u16) -> u16 {
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            std::thread::spawn(move || {
                let mut first = [0u8; 1];
                if c.read_exact(&mut first).is_err() || first[0] == 0x16 {
                    return;
                }
                let Ok(mut h) = TcpStream::connect((Ipv4Addr::LOCALHOST, hub_port)) else {
                    return;
                };
                if h.write_all(&first).is_err() {
                    return;
                }
                let (mut c2, mut h2) = (c.try_clone().unwrap(), h.try_clone().unwrap());
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut c2, &mut h);
                    let _ = h.shutdown(Shutdown::Both);
                });
                let _ = std::io::copy(&mut h2, &mut c);
                let _ = c.shutdown(Shutdown::Both);
            });
        }
    });
    port
}

fn group(id: &str) -> HubFleetTarget {
    HubFleetTarget {
        group: id.into(),
        serials: Vec::new(),
    }
}

fn serials(s: &[&str]) -> HubFleetTarget {
    HubFleetTarget {
        group: String::new(),
        serials: s.iter().map(|x| (*x).to_owned()).collect(),
    }
}

/// The events of `kind` the core has queued, waiting for at least `n`.
fn events(
    core: &DspiCore,
    kind: &str,
    n: usize,
    kept: &mut Vec<HubServerEvent>,
) -> Vec<HubServerEvent> {
    wait_until(kind, || {
        kept.extend(core.hub_server_events());
        kept.iter().filter(|e| e.kind == kind).count() >= n
    });
    kept.iter().filter(|e| e.kind == kind).cloned().collect()
}

// ── Without the caps ────────────────────────────────────────────────────────

#[test]
fn wp_p3cc_without_the_caps_every_call_is_unsupported_and_sends_nothing() {
    let r = rig("nocaps", false);
    let admin = r.core("Bench console", Role::Admin, None);
    let caps = admin.hub_caps();
    assert!(caps.on_hub);
    assert!(
        !caps.groups && !caps.group_apply && !caps.rollout,
        "{caps:?}"
    );
    // `group_params` comes with the parameter API (explicit targets need no
    // groups), so every hub with it announces it: its calls without the cap
    // are the core's FakeHub test (`tests_wp_p3cc.rs`).
    assert!(caps.group_params, "{caps:?}");
    let u = DspiError::Unsupported;
    assert_eq!(admin.hub_groups().unwrap_err(), u);
    assert_eq!(
        admin
            .hub_group_set(HubGroup {
                name: "Bar".into(),
                members: vec![A.into()],
                ..HubGroup::default()
            })
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_group_delete("g1".into()).unwrap_err(), u);
    assert_eq!(
        admin
            .hub_group_apply(
                serials(&[A]),
                "x".into(),
                String::new(),
                String::new(),
                false,
                0
            )
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_group_apply_confirm("a1".into()).unwrap_err(), u);
    assert_eq!(admin.hub_group_apply_cancel("a1".into()).unwrap_err(), u);
    assert_eq!(admin.hub_firmware_images().unwrap_err(), u);
    assert_eq!(
        admin
            .hub_firmware_put(IMAGE.into(), image("1.1.7", 0))
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_firmware_delete(IMAGE.into()).unwrap_err(), u);
    assert_eq!(
        admin
            .hub_rollout_start(serials(&[A]), IMAGE.into(), false)
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_rollouts(String::new()).unwrap_err(), u);
    assert_eq!(admin.hub_rollout_cancel("ro1".into()).unwrap_err(), u);
    admin.link_close();
    r.hub.stop();
}

// ── Groups, group writes, bulk apply ────────────────────────────────────────

#[test]
fn wp_p3cc_groups_and_their_writes_through_the_core() {
    let r = rig("groups", true);
    let admin = r.core("Installer", Role::Admin, None);
    let control = r.core("Front of house", Role::Control, None);
    let viewer = r.core("Lobby screen", Role::Viewer, None);
    let caps = admin.hub_caps();
    assert!(
        caps.groups && caps.group_params && caps.group_apply && caps.rollout,
        "{caps:?}"
    );
    let mut seen = Vec::new();

    // A group: Admin only; every role lists it; group.changed reaches the core.
    let made = admin
        .hub_group_set(HubGroup {
            name: "Hall".into(),
            note: "Ceiling speakers".into(),
            members: vec![A.into(), B.into()],
            ..HubGroup::default()
        })
        .unwrap();
    assert!(made.created);
    let id = made.group.id.clone();
    assert_eq!(id, "g1");
    assert_eq!(made.group.members, [A, B]);
    let ev = events(&admin, "group.changed", 1, &mut seen);
    assert_eq!(ev[0].group.as_ref().unwrap().id, id);
    assert_eq!(ev[0].change, "set");
    assert_eq!(viewer.hub_groups().unwrap(), vec![made.group.clone()]);
    assert_eq!(
        control
            .hub_group_set(HubGroup {
                name: "X".into(),
                ..HubGroup::default()
            })
            .unwrap_err(),
        DspiError::Denied
    );
    let e = admin
        .hub_group_set(HubGroup {
            name: "Bad".into(),
            members: vec!["NO-SUCH".into()],
            ..HubGroup::default()
        })
        .unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    // Replaced whole: no note now.
    let replaced = admin
        .hub_group_set(HubGroup {
            id: id.clone(),
            name: "Hall".into(),
            note: String::new(),
            members: vec![A.into(), B.into()],
        })
        .unwrap();
    assert!(!replaced.created && replaced.group.note.is_empty());

    // Group writes: Control writes with an offset, the Viewer reads; rows in order.
    let set = control
        .hub_group_param_set(
            group(&id),
            "vol.master".into(),
            vec![],
            ParamValue::Float(-20.0),
            vec![HubOffset {
                serial: B.into(),
                db: -3.0,
            }],
        )
        .unwrap();
    assert_eq!(set.path, "vol.master");
    let rows: Vec<(&str, bool, Option<ParamValue>)> = set
        .results
        .iter()
        .map(|r| (r.serial.as_str(), r.ok, r.value.clone()))
        .collect();
    assert_eq!(
        rows,
        [
            (A, true, Some(ParamValue::Float(-20.0))),
            (B, true, Some(ParamValue::Float(-23.0)))
        ],
        "{set:?}"
    );
    let got = viewer
        .hub_group_param_get(group(&id), "vol.master".into(), vec![])
        .unwrap();
    assert_eq!(
        got.results
            .iter()
            .map(|r| r.value.clone())
            .collect::<Vec<_>>(),
        [
            Some(ParamValue::Float(-20.0)),
            Some(ParamValue::Float(-23.0))
        ]
    );
    assert_eq!(
        viewer
            .hub_group_param_set(
                group(&id),
                "vol.master".into(),
                vec![],
                ParamValue::Float(-30.0),
                vec![]
            )
            .unwrap_err(),
        DspiError::Denied
    );
    // A target the hub does not know is that row's not_found, typed.
    let mixed = control
        .hub_group_param_get(serials(&[A, "NO-SUCH"]), "vol.mute".into(), vec![])
        .unwrap();
    assert!(mixed.results[0].ok, "{mixed:?}");
    assert!(
        matches!(mixed.results[0].value, Some(ParamValue::Bool(_))),
        "{mixed:?}"
    );
    assert_eq!(
        (
            mixed.results[1].code.as_str(),
            mixed.results[1].notice_key.as_str()
        ),
        ("not_found", "dlink.not_found")
    );
    // The hub's refusals of a whole request, typed; the caller's own mistakes
    // refused before anything.
    let e = control
        .hub_group_param_set(
            group("g9"),
            "vol.master".into(),
            vec![],
            ParamValue::Float(-9.0),
            vec![],
        )
        .unwrap_err();
    assert_eq!(link_code(&e), "not_found");
    let e = control
        .hub_group_param_set(
            group(&id),
            "vol.mute".into(),
            vec![],
            ParamValue::Bool(true),
            vec![HubOffset {
                serial: A.into(),
                db: 1.0,
            }],
        )
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    assert!(matches!(
        control.hub_group_param_get(HubFleetTarget::default(), "vol.master".into(), vec![]),
        Err(DspiError::InvalidArgument { .. })
    ));

    // Presets by name: a device without one is its row's not_found.
    let loaded = control
        .hub_group_preset_load(
            group(&id),
            None,
            "No such preset".into(),
            vec![HubPresetPick {
                serial: B.into(),
                slot: Some(12),
                name: String::new(),
            }],
        )
        .unwrap_err();
    assert!(
        matches!(loaded, DspiError::InvalidArgument { .. }),
        "{loaded:?}"
    );
    let loaded = control
        .hub_group_preset_load(group(&id), None, "No such preset".into(), vec![])
        .unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(
        loaded.iter().all(|r| !r.ok && r.code == "not_found"),
        "{loaded:?}"
    );

    // Bulk apply with a guard per device, confirmed; then one cancelled.
    let snap = r.snapshot_of_a();
    let run = control
        .hub_group_apply(
            group(&id),
            snap.clone(),
            String::new(),
            String::new(),
            true,
            60_000,
        )
        .unwrap();
    assert_eq!(run.run, "a1");
    assert!(!run.confirm_by.is_empty());
    assert_eq!(run.results.len(), 2);
    assert!(
        run.results.iter().all(|r| r.ok && r.guard.is_some()),
        "{run:?}"
    );
    // Another session's run is not found.
    let e = admin.hub_group_apply_confirm(run.run.clone()).unwrap_err();
    assert_eq!(link_code(&e), "not_found");
    let kept_run = control.hub_group_apply_confirm(run.run.clone()).unwrap();
    assert_eq!(kept_run.results.len(), 2);
    assert!(
        kept_run.results.iter().all(|r| r.ok && !r.reverted),
        "{kept_run:?}"
    );
    let run = control
        .hub_group_apply(serials(&[A]), snap, String::new(), String::new(), true, 0)
        .unwrap();
    let back = control.hub_group_apply_cancel(run.run).unwrap();
    assert!(back.results[0].reverted, "{back:?}");
    assert!(matches!(
        control.hub_group_apply(
            serials(&[A]),
            "x".into(),
            "y".into(),
            String::new(),
            false,
            0
        ),
        Err(DspiError::InvalidArgument { .. })
    ));

    // The client read-only mode: the reads go out, every write is refused here.
    admin.set_client_read_only(true).unwrap();
    assert!(admin.hub_groups().is_ok());
    assert!(
        admin
            .hub_group_param_get(group(&id), "vol.master".into(), vec![])
            .is_ok()
    );
    for e in [
        admin.hub_group_delete(id.clone()).unwrap_err(),
        admin
            .hub_group_param_set(
                group(&id),
                "vol.master".into(),
                vec![],
                ParamValue::Float(-30.0),
                vec![],
            )
            .unwrap_err(),
        admin.hub_group_apply_cancel("a1".into()).unwrap_err(),
        admin
            .hub_firmware_put(IMAGE.into(), image("1.1.7", 0))
            .unwrap_err(),
        admin
            .hub_rollout_start(serials(&[R]), IMAGE.into(), false)
            .unwrap_err(),
    ] {
        assert_eq!(e, DspiError::ReadOnly);
    }
    admin.set_client_read_only(false).unwrap();
    assert!(admin.hub_firmware_images().unwrap().images.is_empty());

    // Deleted: group.changed again, and the list is empty.
    admin.hub_group_delete(id.clone()).unwrap();
    // (the set, the replace, the delete)
    let ev = events(&admin, "group.changed", 3, &mut seen);
    assert_eq!(ev[1].change, "set");
    assert_eq!(ev[2].change, "delete");
    assert_eq!(ev[2].group.as_ref().unwrap().id, id);
    assert!(viewer.hub_groups().unwrap().is_empty());
    let e = admin.hub_group_delete(id).unwrap_err();
    assert_eq!(link_code(&e), "not_found");
    for c in [admin, control, viewer] {
        c.link_close();
    }
    r.hub.stop();
}

// ── The rollout, end to end ─────────────────────────────────────────────────

#[test]
fn wp_p3cc_a_rollout_driven_from_the_core_end_to_end() {
    let r = rig("rollout", true);
    let admin = r.core("Bench console", Role::Admin, None);
    let control = r.core("Front of house", Role::Control, None);
    // Over plaintext, through a proxy that refuses TLS.
    let net_port = r.hub.network().unwrap().port();
    let plain = r.core("Old console", Role::Admin, Some(plaintext_proxy(net_port)));
    assert!(
        plain.hub_caps().rollout,
        "the cap is in every session's hello"
    );
    let bytes = image("1.1.7", 300);
    assert!(
        bytes.len() > 36 * 1024 * 3,
        "several chunks: {}",
        bytes.len()
    );

    // tls_required, typed, and no image byte went: the store is still empty.
    for e in [
        plain.hub_firmware_images().unwrap_err(),
        plain
            .hub_firmware_put(IMAGE.into(), bytes.clone())
            .unwrap_err(),
        plain.hub_rollouts(String::new()).unwrap_err(),
        plain
            .hub_rollout_start(serials(&[R]), IMAGE.into(), true)
            .unwrap_err(),
    ] {
        assert_eq!(link_code(&e), "tls_required", "{e:?}");
        assert_eq!(e.notice_key(), "dlink.tls_required");
    }
    assert!(admin.hub_firmware_images().unwrap().images.is_empty());

    // The upload, in chunks: Admin only.
    assert_eq!(
        control
            .hub_firmware_put(IMAGE.into(), bytes.clone())
            .unwrap_err(),
        DspiError::Denied
    );
    let put = admin.hub_firmware_put(IMAGE.into(), bytes.clone()).unwrap();
    assert!(put.created);
    assert_eq!(put.image.name, IMAGE);
    assert_eq!(put.image.size, bytes.len() as u64);
    assert_eq!(
        (
            put.image.firmware_version.as_str(),
            put.image.platform.as_str()
        ),
        ("1.1.7", "RP2350")
    );
    assert_eq!(put.image.stamp_mv, Some(1200));
    assert_eq!(put.image.added_by, "Bench console");
    let again = admin.hub_firmware_put(IMAGE.into(), bytes.clone()).unwrap();
    assert!(!again.created);
    let list = control.hub_firmware_images().unwrap();
    assert_eq!(list.images, vec![put.image.clone()]);
    assert_eq!((list.max_images, list.max_bytes), (4, 8 * 1024 * 1024));
    // A broken image is the hub's bad_request.
    let e = admin
        .hub_firmware_put("broken.uf2".into(), vec![0x55; 1000])
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");

    // Not muted: refused, typed, nothing touched.
    let e = admin
        .hub_rollout_start(serials(&[R]), IMAGE.into(), false)
        .unwrap_err();
    assert!(matches!(e, DspiError::ConfirmationRequired { .. }), "{e:?}");
    // Not on the hub's USB.
    let e = admin
        .hub_rollout_start(serials(&[A]), IMAGE.into(), true)
        .unwrap_err();
    assert_eq!(e, DspiError::Unsupported);
    assert_eq!(
        control
            .hub_rollout_start(serials(&[R]), IMAGE.into(), true)
            .unwrap_err(),
        DspiError::Denied
    );
    assert!(admin.hub_rollouts(String::new()).unwrap().is_empty());

    // Muted with a group write, then the rollout: followed by its events.
    let muted = admin
        .hub_group_param_set(
            serials(&[R]),
            "vol.mute".into(),
            vec![],
            ParamValue::Bool(true),
            vec![],
        )
        .unwrap();
    assert!(muted.results[0].ok, "{muted:?}");
    let _ = admin.hub_server_events();
    let started = admin
        .hub_rollout_start(serials(&[R]), IMAGE.into(), false)
        .unwrap();
    assert_eq!(
        (started.id.as_str(), started.state.as_str()),
        ("ro1", "running")
    );
    assert_eq!(started.target, R);
    assert_eq!(started.image_version, "1.1.7");
    assert_eq!(started.by, "Bench console");
    let steps: Vec<&str> = started.devices[0]
        .steps
        .iter()
        .map(|s| s.step.as_str())
        .collect();
    assert_eq!(steps, ["check", "guard"]);
    assert_eq!(started.devices[0].steps[0].notice_key, "rollout.check");

    let mut seen = Vec::new();
    wait_until("the rollout's end", || {
        seen.extend(admin.hub_server_events());
        seen.iter()
            .any(|e| e.rollout.as_ref().is_some_and(|r| r.state != "running"))
    });
    let changes: Vec<&HubServerEvent> = seen
        .iter()
        .filter(|e| e.kind == "fw.rollout.changed")
        .collect();
    assert!(changes.len() >= 6, "{}", changes.len());
    assert!(
        changes.iter().all(|e| e.serial == R),
        "the delta names the device"
    );
    let done = admin.hub_rollouts("ro1".into()).unwrap().remove(0);
    assert_eq!(done.state, "ok", "{done:?}");
    assert!(!done.ended_at.is_empty());
    let d = &done.devices[0];
    assert_eq!(d.state, "ok");
    let keys: Vec<&str> = d.steps.iter().map(|s| s.notice_key.as_str()).collect();
    assert_eq!(
        keys,
        dspi_core::hub_server::ROLLOUT_STEPS
            .iter()
            .map(|s| format!("rollout.{s}"))
            .collect::<Vec<_>>()
    );
    assert!(d.steps[3].session.is_some(), "the entry's child session");
    let before = d.before.as_ref().unwrap();
    assert_eq!((before.fw.as_str(), before.required_mv), ("1.1.6", 1200));
    assert!(!before.export.is_empty());
    let after = d.after.as_ref().unwrap();
    assert_eq!(after.fw, "1.1.7");
    assert_eq!(after.vreg.unwrap().live_mv, Some(1200));
    assert!(d.failed.is_none());
    assert_eq!(
        r.bench.flashes.lock().unwrap().clone(),
        [(R.to_owned(), bytes.len())]
    );
    // A finished rollout cannot be cancelled; an image it used can go now.
    let e = admin.hub_rollout_cancel("ro1".into()).unwrap_err();
    assert_eq!(link_code(&e), "conflict");
    // The alarm the rollout raises on a failure is listed now, always on.
    let rules = admin.hub_alarm_rules_get().unwrap();
    assert!(
        rules.iter().any(|r| r.code == "rollout_failed"),
        "{rules:?}"
    );
    // ... and its `on` stays true whatever alarm.rules.set says.
    let mut off = rules
        .iter()
        .find(|r| r.code == "rollout_failed")
        .unwrap()
        .clone();
    assert!(off.on);
    off.on = false;
    let after = admin.hub_alarm_rules_set(vec![off]).unwrap();
    assert!(
        after.iter().any(|r| r.code == "rollout_failed" && r.on),
        "{after:?}"
    );
    admin.hub_firmware_delete(IMAGE.into()).unwrap();
    let e = admin.hub_firmware_delete(IMAGE.into()).unwrap_err();
    assert_eq!(link_code(&e), "not_found");
    no_base64_in_logs();
    for c in [admin, control, plain] {
        c.link_close();
    }
    r.hub.stop();
}
