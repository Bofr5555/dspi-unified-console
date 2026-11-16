//! WP P3RO's done criteria through the real server: the firmware rollout (`fw.*`,
//! `hub-api.md` section 21) on demo devices that the hub reaches as if over its USB, a
//! fake BOOTSEL scan and a fake writer injected through `ServiceConfig::rollout`, the test
//! clock for the waits, and clients over TLS (the hub runs `--tls on` here whatever
//! `DSPI_HUB_TEST_TLS` says, so the suite is the same in `off`, `on` and `compat`; the
//! plaintext refusal is shown in `on`, in upstream compatibility and on a `--tls off` hub).
//!
//! No USB, no real device: the "device" is a demo `FakeDevice` behind a wrapper that
//! notes `REQ_ENTER_BOOTLOADER`, and "the board in BOOTSEL" is what the fake scan says.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_core::{BootDevice, FirmwareGuardBoard, FlashReport};
use dspi_hub::rollout::{BootScan, RolloutConfig, Writer};
use dspi_hub::session::{FIRST_REMOTE_SESSION_ID, GUARD_SESSION_ID, INTERNAL_SESSION_ID};
use dspi_hub::{
    AccessMode, HistoryConfig, Hub, HubConfig, HubHandle, NetworkConfig, Role, ServiceConfig,
    TestClock, TlsMode,
};
use dspi_link::tls::{self as link_tls, PinMode, TlsPipe};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, NotificationSource, Transport,
    TransportCaps,
};
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(30);
/// The DSPi's USB vendor id: a device "on this hub's USB".
const VID: u16 = 0x2E8B;
const T0: u64 = 1_800_000_000;

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

/// No run of 100 or more base64 characters in anything logged: no image bytes (a
/// SHA-256 in hex, 64 characters, is fine).
fn no_base64_in_logs() {
    let logs = kept().lock().unwrap().clone();
    for line in &logs {
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
    let dir = std::env::temp_dir().join("wp-p3ro").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// An RP2350 DSPi image of `version`, stamped `stamp` mV (`None`: unstamped).
fn image(version: &str, stamp: Option<&str>) -> Vec<u8> {
    let mut entries: Vec<(u16, u32, String)> = vec![
        (TAG_RP, ID_NAME, "DSPi".into()),
        (TAG_RP, ID_VERSION, version.into()),
    ];
    if let Some(s) = stamp {
        entries.push((TAG_DS, ID_STAMP, s.into()));
    }
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

// ── The device: a demo noting its bootloader entry ──────────────────────────

/// A device's side, for the test: whether it was sent to its bootloader, and a gate that
/// holds its reads (to catch a rollout between two steps).
#[derive(Default)]
struct Side {
    entered: AtomicBool,
    hold: Mutex<bool>,
    released: Condvar,
}

impl Side {
    fn hold(&self, on: bool) {
        *self.hold.lock().unwrap() = on;
        self.released.notify_all();
    }

    fn wait_released(&self) {
        let mut h = self.hold.lock().unwrap();
        let start = Instant::now();
        while *h && start.elapsed() < WAIT {
            h = self
                .released
                .wait_timeout(h, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }
}

struct Dev {
    inner: FakeDevice,
    side: Arc<Side>,
}

impl Transport for Dev {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.side.wait_released();
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.side.entered.store(true, Ordering::SeqCst);
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
        self.side.wait_released();
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.side.entered.store(true, Ordering::SeqCst);
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

/// The demo device reporting firmware `1.1.<patch>`, its build default core voltage
/// `vreg` (0x0D 1.20 V, 0x0C 1.15 V).
fn demo(serial: &str, patch: u8, vreg: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    // `GET_PLATFORM`: major, (minor << 4) | patch, outputs, minor, patch, beta.
    capture.platform[2] = 0x10 | (patch & 0x0F);
    capture.platform[5] = patch;
    let options = FakeOptions {
        extensions: true,
        surfaces: true,
        analysis: true,
        ..FakeOptions::default()
    };
    let d = FakeDevice::new(capture, options).with_serial(serial);
    d.remote().vreg_set_build_default(vreg);
    d
}

// ── The fake BOOTSEL scan and writer ────────────────────────────────────────

/// What a device does after its flash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Back {
    /// Comes back with firmware 1.1.<patch> and core default `vreg`.
    As(u8, u8),
    /// Never comes back.
    Never,
}

struct Plan {
    /// Boards in BOOTSEL before any entry.
    before: usize,
    /// Boards that appear after a device's entry (1 normally).
    after: usize,
    /// What a device does after its flash (default `As(7, 0x0D)`).
    back: Vec<(String, Back)>,
    /// The writer holds until released.
    hold_flash: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            before: 0,
            after: 1,
            back: Vec::new(),
            hold_flash: false,
        }
    }
}

#[derive(Default)]
struct Bench {
    hub: Mutex<Option<HubHandle>>,
    plan: Mutex<Plan>,
    /// Each attachment's side, by serial, newest last.
    sides: Mutex<Vec<(String, Arc<Side>)>>,
    /// The device sent to its bootloader and not yet flashed.
    in_boot: Mutex<Option<String>>,
    flashes: Mutex<Vec<(String, FirmwareGuardBoard, usize)>>,
    flash_gate: Mutex<bool>,
    flash_released: Condvar,
}

impl Bench {
    fn hub(&self) -> HubHandle {
        self.hub.lock().unwrap().clone().unwrap()
    }

    fn side(&self, serial: &str) -> Arc<Side> {
        let sides = self.sides.lock().unwrap();
        Arc::clone(&sides.iter().rev().find(|(s, _)| s == serial).unwrap().1)
    }

    fn plug_with(&self, serial: &str, inner: FakeDevice, vid: u16) -> u8 {
        let side = Arc::new(Side::default());
        self.sides
            .lock()
            .unwrap()
            .push((serial.to_owned(), Arc::clone(&side)));
        self.hub()
            .attach(Box::new(Dev { inner, side }), vid)
            .unwrap()
    }

    fn plug_as(&self, serial: &str, patch: u8, vreg: u8, vid: u16) -> u8 {
        self.plug_with(serial, demo(serial, patch, vreg), vid)
    }

    fn plug(&self, serial: &str) -> u8 {
        self.plug_as(serial, 6, 0x0D, VID)
    }

    fn handle_of(&self, serial: &str) -> Option<u8> {
        self.hub()
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial && d.state == dspi_hub::DeviceState::Online)
            .map(|d| d.handle)
    }

    fn board(serial: &str, n: usize) -> BootDevice {
        BootDevice {
            platform: 1,
            serial: Some(if n == 0 {
                serial.to_owned()
            } else {
                format!("OTHER{n}")
            }),
            bus_location: format!("1-{}", n + 2),
        }
    }

    /// Hold every flash until [`Bench::release_flash`]. The gate is armed here, before the
    /// writer is entered, so a release can never come before the writer waits on it.
    fn hold_flashes(&self) {
        *self.flash_gate.lock().unwrap() = true;
        self.plan.lock().unwrap().hold_flash = true;
    }

    fn release_flash(&self) {
        *self.flash_gate.lock().unwrap() = false;
        self.flash_released.notify_all();
    }

    fn flashed(&self) -> usize {
        self.flashes.lock().unwrap().len()
    }

    fn entered(&self, serial: &str) -> bool {
        self.sides
            .lock()
            .unwrap()
            .iter()
            .any(|(s, side)| s == serial && side.entered.load(Ordering::SeqCst))
    }
}

impl BootScan for Bench {
    fn boards(&self) -> Vec<BootDevice> {
        // A device that took its bootloader entry leaves the bus, and its boot ROM shows.
        let entered = {
            let sides = self.sides.lock().unwrap();
            sides
                .iter()
                .filter(|(_, side)| side.entered.load(Ordering::SeqCst))
                .map(|(s, _)| s.clone())
                .next_back()
        };
        if let Some(serial) = entered.or_else(|| self.in_boot.lock().unwrap().clone()) {
            if let Some(h) = self.handle_of(&serial) {
                let _ = self.hub().detach(h);
            }
            *self.in_boot.lock().unwrap() = Some(serial.clone());
            let n = self.plan.lock().unwrap().after;
            return (0..n).map(|i| Self::board(&serial, i)).collect();
        }
        let n = self.plan.lock().unwrap().before;
        (0..n).map(|i| Self::board("SOMEONE", i + 1)).collect()
    }
}

impl Writer for Bench {
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        let serial = self.in_boot.lock().unwrap().clone().unwrap_or_default();
        self.flashes
            .lock()
            .unwrap()
            .push((serial.clone(), board.clone(), image.len()));
        if self.plan.lock().unwrap().hold_flash {
            // Armed by `hold_flashes`; released (false) by `release_flash`.
            let mut g = self.flash_gate.lock().unwrap();
            let start = Instant::now();
            while *g && start.elapsed() < WAIT {
                g = self
                    .flash_released
                    .wait_timeout(g, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        }
        // Out of BOOTSEL: the entry is spent.
        *self.in_boot.lock().unwrap() = None;
        for (s, side) in self.sides.lock().unwrap().iter() {
            if *s == serial {
                side.entered.store(false, Ordering::SeqCst);
            }
        }
        let back = self
            .plan
            .lock()
            .unwrap()
            .back
            .iter()
            .find(|(s, _)| *s == serial)
            .map_or(Back::As(7, 0x0D), |(_, b)| *b);
        if let Back::As(patch, vreg) = back {
            self.plug_as(&serial, patch, vreg, VID);
        }
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
    clock: Arc<TestClock>,
    bench: Arc<Bench>,
    dir: PathBuf,
}

struct Opts {
    tls: TlsMode,
    compat: bool,
    allow_unverified: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            tls: TlsMode::On,
            compat: false,
            allow_unverified: true,
        }
    }
}

fn rig(tag: &str) -> Rig {
    rig_with(tag, Opts::default())
}

fn rig_with(tag: &str, o: Opts) -> Rig {
    keep_logs();
    let dir = scratch(tag);
    let clock = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(T0)));
    let bench = Arc::new(Bench::default());
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tls: o.tls,
            upstream_compat: o.compat,
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            clock: Arc::clone(&clock) as Arc<dyn dspi_hub::HubClock>,
            inventory: true,
            groups: true,
            alarms: true,
            history: Some(HistoryConfig::default()),
            rollout: Some(RolloutConfig {
                allow_unverified_firmware: o.allow_unverified,
                boot_scan: Some(Arc::clone(&bench) as Arc<dyn BootScan>),
                writer: Some(Arc::clone(&bench) as Arc<dyn Writer>),
                // The hub sets it from its TLS mode: this is overwritten.
                serves_tls: true,
            }),
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p3ro")
    })
    .unwrap();
    *bench.hub.lock().unwrap() = Some(hub.handle());
    Rig {
        hub,
        clock,
        bench,
        dir,
    }
}

impl Rig {
    fn store(&self) -> PathBuf {
        self.dir.join("store")
    }

    /// A paired client with `role`, over TLS (pinned) or plaintext.
    fn client(&self, name: &str, role: Role, tls: bool) -> Ws {
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
        let conn = if tls {
            let deadline = Instant::now() + Duration::from_secs(5);
            let pin = net.tls().pin.expect("TLS is served");
            let (mut pipe, _) =
                link_tls::connect(raw(net.port()), "localhost", PinMode::Pinned(pin), deadline)
                    .unwrap();
            pipe.set_deadline(None);
            Conn::Tls(upgrade(pipe, net.port()))
        } else {
            Conn::Plain(upgrade(raw(net.port()), net.port()))
        };
        let mut ws = Ws {
            conn,
            next_id: 100,
            caps: Vec::new(),
            seen: Vec::new(),
        };
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": name, "app": "wp_p3ro", "version": "0"}}));
        let hello = ws.next("hello", |_| true);
        ws.caps = hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws
    }

    fn admin(&self) -> Ws {
        self.client("Bench console", Role::Admin, true)
    }

    fn advance(&self, by: Duration) {
        self.clock.advance(by);
        self.hub.handle().wake_services();
    }
}

fn raw(port: u16) -> TcpStream {
    let tcp = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp.set_write_timeout(Some(WAIT)).unwrap();
    tcp
}

fn upgrade<S: Read + Write>(mut s: S, port: u16) -> WebSocket<S> {
    let req = format!(
        "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
    );
    s.write_all(req.as_bytes()).unwrap();
    s.flush().unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => head.push(byte[0]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    WebSocket::from_raw_socket(s, WsRole::Client, None)
}

enum Conn {
    Tls(WebSocket<TlsPipe>),
    Plain(WebSocket<TcpStream>),
}

struct Ws {
    conn: Conn,
    next_id: u64,
    caps: Vec<String>,
    seen: Vec<Value>,
}

impl Ws {
    fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c == cap)
    }

    fn send(&mut self, v: Value) {
        let m = Message::text(v.to_string());
        match &mut self.conn {
            Conn::Tls(w) => w.send(m).unwrap(),
            Conn::Plain(w) => w.send(m).unwrap(),
        }
    }

    fn read(&mut self) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no text frame in time");
            let m = match &mut self.conn {
                Conn::Tls(w) => w.read().unwrap(),
                Conn::Plain(w) => w.read().unwrap(),
            };
            if let Message::Text(text) = m {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn next(&mut self, t: &str, pick: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.seen.iter().position(|v| v["t"] == t && pick(v)) {
            return self.seen.remove(i);
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            let v = self.read();
            if v["t"] == t && pick(&v) {
                return v;
            }
            self.seen.push(v);
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.send(msg);
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            let v = self.read();
            if v["id"] == id && (v["t"] == "ok" || v["t"] == "err") {
                return v;
            }
            self.seen.push(v);
        }
    }

    /// Every `fw.rollout.changed` seen so far, drained.
    fn changes(&mut self) -> Vec<Value> {
        let (mine, rest): (Vec<Value>, Vec<Value>) = std::mem::take(&mut self.seen)
            .into_iter()
            .partition(|v| v["t"] == "fw.rollout.changed");
        self.seen = rest;
        mine
    }

    /// Put `bytes` as image `name` in chunks: the last answer.
    fn put_image(&mut self, name: &str, bytes: &[u8]) -> Value {
        let enc = data_encoding::BASE64.encode(bytes);
        let chunks: Vec<&str> = enc
            .as_bytes()
            .chunks(49_152)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        let mut last = Value::Null;
        for (i, c) in chunks.iter().enumerate() {
            last = self.ask(json!({"t": "fw.image.put", "name": name, "seq": i,
                "data": c, "last": i + 1 == chunks.len()}));
            if last["t"] != "ok" {
                return last;
            }
        }
        last
    }

    /// Start a rollout of `dspi-1.1.7.uf2`: the answer.
    fn start(&mut self, target: Value) -> Value {
        let mut msg = json!({"t": "fw.rollout.start", "image": "dspi-1.1.7.uf2"});
        for (k, v) in target.as_object().unwrap() {
            msg[k] = v.clone();
        }
        self.ask(msg)
    }

    fn record(&mut self, id: &str) -> Value {
        let v = self.ask(json!({"t": "fw.rollout.status", "rollout": id}));
        v["rollouts"][0].clone()
    }

    /// The record of `id` once it is no longer running.
    fn ended(&mut self, id: &str) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "rollout {id} did not end");
            let r = self.record(id);
            if r["state"] != "running" {
                return r;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait until rollout `id`'s device `i` is at `step`.
    fn at_step(&mut self, id: &str, i: usize, step: &str) {
        let start = Instant::now();
        loop {
            let r = self.record(id);
            assert!(start.elapsed() < WAIT, "rollout {id} never at {step}: {r}");
            if r["devices"][i]["step"] == step {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn alarms(&mut self) -> Vec<Value> {
        let v = self.ask(json!({"t": "alarm.list"}));
        v["alarms"].as_array().cloned().unwrap_or_default()
    }

    fn rollout_alarm(&mut self, serial: &str) -> Option<Value> {
        self.alarms()
            .into_iter()
            .find(|a| a["code"] == "rollout_failed" && a["serial"] == serial)
    }
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn ok(v: &Value) -> bool {
    v["t"] == "ok"
}

/// A rig with device `serial` plugged and the 1.20 V image `dspi-1.1.7.uf2` stored.
fn ready(tag: &str, serial: &str) -> (Rig, Ws) {
    let r = rig(tag);
    r.bench.plug(serial);
    let mut admin = r.admin();
    let put = admin.put_image("dspi-1.1.7.uf2", &image("1.1.7", Some("1200")));
    assert!(ok(&put), "{put}");
    (r, admin)
}

fn steps_of(device: &Value) -> Vec<String> {
    device["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["step"].as_str().unwrap().to_owned())
        .collect()
}

fn states_of(record: &Value) -> Vec<String> {
    record["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["state"].as_str().unwrap().to_owned())
        .collect()
}

// ── A full rollout ──────────────────────────────────────────────────────────

#[test]
fn wp_p3ro_a_full_rollout_succeeds_with_every_step_recorded() {
    let (r, mut admin) = ready("full", "P3RO-FULL");
    assert!(admin.has_cap("rollout"));
    let serial = "P3RO-FULL";
    // Muted first: no while_live needed.
    let muted = admin.ask(
        json!({"t": "param.set", "serial": serial, "path": "vol.mute",
        "value": true}),
    );
    assert!(ok(&muted), "{muted}");
    let started = admin.start(json!({"serial": serial}));
    assert!(ok(&started), "{started}");
    assert_eq!(started["v"], 1);
    let rec = &started["rollout"];
    assert_eq!(rec["state"], "running");
    assert_eq!(rec["by"], "Bench console");
    assert_eq!(rec["target"], serial);
    assert_eq!(rec["while_live"], false);
    assert_eq!(rec["image"]["firmware_version"], "1.1.7");
    assert_eq!(steps_of(&rec["devices"][0]), ["check", "guard"]);
    let id = rec["id"].as_str().unwrap().to_owned();
    assert_eq!(id, "ro1");

    let done = admin.ended(&id);
    assert_eq!(done["state"], "ok", "{done}");
    assert!(done["ended_at"].is_string());
    let d = &done["devices"][0];
    assert_eq!(d["state"], "ok", "{d}");
    assert_eq!(
        steps_of(d),
        [
            "check",
            "guard",
            "export",
            "bootloader",
            "find",
            "flash",
            "wait",
            "verify"
        ]
    );
    assert!(
        d["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["ok"] == true)
    );
    assert_eq!(d["before"]["board"]["required_mv"], 1200);
    assert_eq!(d["before"]["board"]["source"], "extCoreVreg");
    assert_eq!(d["before"]["vreg"]["stored_mv"], 1200);
    assert_eq!(d["before"]["fw"], "1.1.6");
    assert!(!d["before"]["ext_list"].as_array().unwrap().is_empty());
    assert_eq!(d["after"]["fw"], "1.1.7");
    assert_eq!(d["after"]["vreg"]["live_mv"], 1200);
    assert_eq!(d["after"]["vreg"]["stored_mv"], 1200);
    assert_eq!(d["after"]["settings"], "unchanged", "{d}");
    let verify = d["steps"][7]["text"].as_str().unwrap();
    assert!(verify.contains("core voltage 1.20 V read back"), "{verify}");
    assert!(d["failed"].is_null());

    // The bootloader entry went out as a child session from the remote range.
    let session = u16::try_from(d["steps"][3]["session"].as_u64().unwrap()).unwrap();
    assert!(session >= FIRST_REMOTE_SESSION_ID, "{session}");
    assert!(![INTERNAL_SESSION_ID, GUARD_SESSION_ID].contains(&session));
    // The writer got the guard's board record from before BOOTSEL, and the image.
    let flashes = r.bench.flashes.lock().unwrap().clone();
    assert_eq!(flashes.len(), 1);
    assert_eq!(flashes[0].0, serial);
    assert_eq!(flashes[0].1.required_mv, 1200);
    assert_eq!(flashes[0].2, image("1.1.7", Some("1200")).len());

    // The export is in the hub's snapshot store, by its name.
    let export = d["before"]["export"].as_str().unwrap();
    let list = admin.ask(json!({"t": "snapshot.list", "serial": serial}));
    let snap = list["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == export)
        .cloned()
        .unwrap_or_else(|| panic!("{list}"));
    assert!(
        snap["name"]
            .as_str()
            .unwrap()
            .starts_with("Before firmware 1.1.7, 20"),
        "{snap}"
    );

    // The history's `firmware` record names the Admin.
    let start = Instant::now();
    let record = loop {
        let events = admin.ask(json!({"t": "event.list", "serial": serial, "limit": 100}));
        let found = events["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "firmware" && e["source"] == "rollout")
            .cloned();
        if let Some(f) = found {
            break f;
        }
        assert!(start.elapsed() < WAIT, "{events}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(record["by"], "Bench console", "{record}");
    assert!(record["text"].as_str().unwrap().contains("1.1.7"));

    // Every step was announced to the encrypted session.
    let changes = admin.changes();
    assert!(changes.len() >= 8, "{}", changes.len());
    assert!(changes.iter().all(|c| c["v"] == 1 && c["hub"].is_string()));
    assert!(changes.iter().any(|c| c["rollout"]["state"] == "ok"));
    assert!(admin.rollout_alarm(serial).is_none());
    // The files are the hub's alone, and no image byte went into a log.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for f in [
            r.store().join("rollout.json"),
            r.store().join("firmware").join("dspi-1.1.7.uf2"),
            r.store().join("firmware").join("index.json"),
        ] {
            let mode = std::fs::metadata(&f).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", f.display());
        }
    }
    no_base64_in_logs();
    r.hub.stop();
}

// ── Refused early ───────────────────────────────────────────────────────────

/// A refused start: its code, no record, no alarm, nothing sent to BOOTSEL or written.
fn refused(r: &Rig, admin: &mut Ws, serial: &str, v: &Value, want: &str) {
    assert_eq!(code(v), want, "{v}");
    assert_eq!(v["v"], 1);
    let st = admin.ask(json!({"t": "fw.rollout.status"}));
    assert_eq!(st["rollouts"], json!([]), "a refused start left a record");
    assert!(admin.rollout_alarm(serial).is_none());
    assert_eq!(r.bench.flashed(), 0);
    assert!(!r.bench.entered(serial), "the device was sent to BOOTSEL");
}

#[test]
fn wp_p3ro_roles_and_plaintext_are_refused() {
    let (r, mut admin) = ready("roles", "P3RO-ROLES");
    let serial = "P3RO-ROLES";
    for role in [Role::Control, Role::Viewer] {
        let mut c = r.client(&format!("{role:?} console"), role, true);
        let v = c.start(json!({"serial": serial, "while_live": true}));
        refused(&r, &mut admin, serial, &v, "denied");
        let put = c.ask(
            json!({"t": "fw.image.put", "name": "x.uf2", "seq": 0, "data": "",
            "last": true}),
        );
        assert_eq!(code(&put), "denied", "{put}");
        let del = c.ask(json!({"t": "fw.image.delete", "image": "dspi-1.1.7.uf2"}));
        assert_eq!(code(&del), "denied");
        let cancel = c.ask(json!({"t": "fw.rollout.cancel", "rollout": "ro1"}));
        assert_eq!(code(&cancel), "denied");
        // Reads are every role's.
        let list = c.ask(json!({"t": "fw.image.list"}));
        assert!(ok(&list), "{list}");
        assert_eq!(list["images"][0]["name"], "dspi-1.1.7.uf2");
        assert_eq!(list["max_images"], 4);
        assert_eq!(list["max_bytes"], 8_388_608);
        assert!(ok(&c.ask(json!({"t": "fw.rollout.status"}))));
    }
    // Plaintext: every fw.* is tls_required, the Admin's too, the reads too.
    let mut plain = r.client("Plain admin", Role::Admin, false);
    assert!(
        plain.has_cap("rollout"),
        "the cap is in a plaintext hello too"
    );
    for msg in [
        json!({"t": "fw.image.list"}),
        json!({"t": "fw.rollout.status"}),
        json!({"t": "fw.image.put", "name": "a.uf2", "seq": 0, "data": "", "last": false}),
        json!({"t": "fw.image.delete", "image": "dspi-1.1.7.uf2"}),
        json!({"t": "fw.rollout.start", "serial": serial, "image": "dspi-1.1.7.uf2",
            "while_live": true}),
        json!({"t": "fw.rollout.cancel", "rollout": "ro1"}),
    ] {
        let v = plain.ask(msg.clone());
        refused(&r, &mut admin, serial, &v, "tls_required");
    }
    // The image is still there: the plaintext delete did nothing.
    let list = admin.ask(json!({"t": "fw.image.list"}));
    assert_eq!(list["images"].as_array().unwrap().len(), 1);
    r.hub.stop();
}

#[test]
fn wp_p3ro_plaintext_is_refused_in_upstream_compat_and_a_tls_off_hub_has_no_rollout() {
    let r = rig_with(
        "compat",
        Opts {
            compat: true,
            ..Opts::default()
        },
    );
    let mut plain = r.client("Upstream console", Role::Admin, false);
    let v = plain.ask(json!({"t": "fw.image.list"}));
    assert_eq!(code(&v), "tls_required", "{v}");
    let mut tls = r.client("TLS console", Role::Admin, true);
    assert!(ok(&tls.ask(json!({"t": "fw.image.list"}))));
    r.hub.stop();

    let off = rig_with(
        "off",
        Opts {
            tls: TlsMode::Off,
            ..Opts::default()
        },
    );
    let mut plain = off.client("Console", Role::Admin, false);
    assert!(!plain.has_cap("rollout"), "a --tls off hub has no rollout");
    for t in ["fw.image.list", "fw.rollout.status"] {
        let v = plain.ask(json!({"t": t}));
        assert_eq!(code(&v), "tls_required", "{v}");
    }
    off.hub.stop();
}

#[test]
fn wp_p3ro_a_start_is_refused_early_for_each_condition() {
    let (r, mut admin) = ready("early", "P3RO-EARLY");
    let serial = "P3RO-EARLY";
    let live = |s: &str| json!({"serial": s, "while_live": true});

    // Unmuted without while_live.
    let v = admin.start(json!({"serial": serial}));
    refused(&r, &mut admin, serial, &v, "confirmation_required");
    // Not on this hub's USB (vid 0).
    r.bench.plug_as("P3RO-NOUSB", 6, 0x0D, 0);
    let v = admin.start(live("P3RO-NOUSB"));
    refused(&r, &mut admin, "P3RO-NOUSB", &v, "unsupported");
    // Unknown, then offline.
    let v = admin.start(live("P3RO-NOBODY"));
    refused(&r, &mut admin, "P3RO-NOBODY", &v, "not_found");
    let h = r.bench.plug("P3RO-GONE");
    r.hub.detach(h).unwrap();
    let v = admin.start(live("P3RO-GONE"));
    refused(&r, &mut admin, "P3RO-GONE", &v, "offline");
    // Another session's device lock.
    let handle = r.bench.handle_of(serial).unwrap();
    let other = r.hub.open_session(Role::Control, None).unwrap();
    assert!(r.hub.lock_acquire(&other, handle, Duration::from_secs(60)));
    let v = admin.start(live(serial));
    refused(&r, &mut admin, serial, &v, "locked");
    r.hub.lock_release(&other, handle);
    // A pending guarded apply.
    r.hub
        .apply_begin(&other, handle, Duration::from_secs(60))
        .unwrap();
    let v = admin.start(live(serial));
    refused(&r, &mut admin, serial, &v, "conflict");
    r.hub.apply_cancel(&other, handle).unwrap();
    r.hub.close_session(other.id());
    // A board already in BOOTSEL.
    r.bench.plan.lock().unwrap().before = 1;
    let v = admin.start(live(serial));
    refused(&r, &mut admin, serial, &v, "conflict");
    assert!(v["msg"].as_str().unwrap().contains("BOOTSEL"), "{v}");
    r.bench.plan.lock().unwrap().before = 0;
    // No such image.
    let mut msg = live(serial);
    msg["t"] = "fw.rollout.start".into();
    msg["image"] = "nothing.uf2".into();
    let v = admin.ask(msg);
    refused(&r, &mut admin, serial, &v, "not_found");
    // An installer lock (a PIN and what it covers) does not stop an Admin's rollout: the
    // Admin's principal passes it, as an Admin's write does (PLAN-P3 step 4).
    let set = admin.ask(json!({"t": "limits.set", "handle": handle,
        "profile": {"lockedPages": ["eq"]}}));
    assert!(ok(&set), "{set}");
    let pin = admin.ask(json!({"t": "limits.pin", "handle": handle, "pin": "2468"}));
    assert!(ok(&pin), "{pin}");
    let v = admin.start(live(serial));
    assert!(ok(&v), "{v}");
    let id = v["rollout"]["id"].as_str().unwrap().to_owned();
    assert_eq!(admin.ended(&id)["state"], "ok", "under an installer lock");
    let unpin = admin.ask(json!({"t": "limits.pin", "handle": handle, "pin": null}));
    assert!(ok(&unpin), "{unpin}");
    // With nothing in the way it goes.
    let v = admin.start(live(serial));
    assert!(ok(&v), "{v}");
    let id = v["rollout"]["id"].as_str().unwrap().to_owned();
    assert_eq!(admin.ended(&id)["state"], "ok");
    // A second start while one runs is busy; its image is not deleted meanwhile.
    r.bench.hold_flashes();
    let v = admin.start(live(serial));
    assert!(ok(&v), "{v}");
    let id = v["rollout"]["id"].as_str().unwrap().to_owned();
    admin.at_step(&id, 0, "flash");
    let busy = admin.start(live(serial));
    assert_eq!(code(&busy), "busy", "{busy}");
    let del = admin.ask(json!({"t": "fw.image.delete", "image": "dspi-1.1.7.uf2"}));
    assert_eq!(code(&del), "conflict", "{del}");
    r.bench.release_flash();
    assert_eq!(admin.ended(&id)["state"], "ok");
    r.hub.stop();
}

#[test]
fn wp_p3ro_images_the_guard_or_the_manifest_would_ask_for_are_refused() {
    let (r, mut admin) = ready("guard", "P3RO-GUARD");
    let serial = "P3RO-GUARD";
    let try_image = |admin: &mut Ws, name: &str, bytes: Vec<u8>, serial: &str| {
        let put = admin.put_image(name, &bytes);
        assert!(ok(&put), "{put}");
        admin.ask(
            json!({"t": "fw.rollout.start", "serial": serial, "image": name,
            "while_live": true}),
        )
    };
    // Stamped 1250 on a 1200 board: the OVERVOLT row.
    let v = try_image(&mut admin, "over.uf2", image("1.1.7", Some("1250")), serial);
    refused(&r, &mut admin, serial, &v, "image_refused");
    assert_eq!(v["reason"], "overvolt", "{v}");
    assert!(
        v["msg"].as_str().unwrap().contains("1.25 V"),
        "the guard's message: {v}"
    );
    // Unstamped on a 1200 board: the phrase row.
    let v = try_image(&mut admin, "plain.uf2", image("1.1.7", None), serial);
    refused(&r, &mut admin, serial, &v, "image_refused");
    assert_eq!(v["reason"], "image_unknown", "{v}");
    assert!(v["msg"].as_str().unwrap().contains("1.20 V"), "{v}");
    // Stamped lower.
    assert!(ok(
        &admin.ask(json!({"t": "fw.image.delete", "image": "over.uf2"}))
    ));
    let v = try_image(&mut admin, "low.uf2", image("1.1.7", Some("1150")), serial);
    refused(&r, &mut admin, serial, &v, "image_refused");
    assert_eq!(v["reason"], "image_lower", "{v}");
    // A board whose needs are not known (no extension channel, no voltage in its status
    // block): any image.
    // (A core setting outside the known codes: no voltage the guard can believe.)
    r.bench.plug_as("P3RO-UNKN", 6, 0x05, VID);
    let v = admin.ask(json!({"t": "fw.rollout.start", "serial": "P3RO-UNKN",
        "image": "dspi-1.1.7.uf2", "while_live": true}));
    refused(&r, &mut admin, "P3RO-UNKN", &v, "image_refused");
    assert_eq!(v["reason"], "board_unknown", "{v}");
    r.hub.stop();

    // Without --allow-unverified-firmware a test image (not in the manifest) is refused.
    let strict = rig_with(
        "strict",
        Opts {
            allow_unverified: false,
            ..Opts::default()
        },
    );
    strict.bench.plug("P3RO-STRICT");
    let mut admin = strict.admin();
    let put = admin.put_image("dspi-1.1.7.uf2", &image("1.1.7", Some("1200")));
    assert!(ok(&put), "{put}");
    assert_eq!(put["image"]["verified"], false);
    assert_eq!(put["image"]["stamp_mv"], 1200);
    assert_eq!(put["image"]["platform"], "RP2350");
    let v = admin.start(json!({"serial": "P3RO-STRICT", "while_live": true}));
    refused(&strict, &mut admin, "P3RO-STRICT", &v, "image_refused");
    assert_eq!(v["reason"], "not_verified", "{v}");
    strict.hub.stop();
}

// ── Failures after the start ────────────────────────────────────────────────

/// A rollout that failed at `step`: its device's entry, with `rollout_failed` raised.
fn failed_at(admin: &mut Ws, id: &str, serial: &str, step: &str) -> Value {
    let done = admin.ended(id);
    assert_eq!(done["state"], "failed", "{done}");
    let d = done["devices"][0].clone();
    assert_eq!(d["state"], "failed", "{d}");
    assert_eq!(d["failed"]["step"], step, "{d}");
    let start = Instant::now();
    let alarm = loop {
        if let Some(a) = admin.rollout_alarm(serial) {
            break a;
        }
        assert!(start.elapsed() < WAIT, "no rollout_failed alarm");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(alarm["severity"], "critical", "{alarm}");
    assert_eq!(alarm["state"], "raised");
    assert!(alarm["message"].as_str().unwrap().contains(step), "{alarm}");
    d
}

#[test]
fn wp_p3ro_two_boards_in_bootsel_after_the_entry_fail_with_no_write() {
    let (r, mut admin) = ready("two", "P3RO-TWO");
    r.bench.plan.lock().unwrap().after = 2;
    let v = admin.start(json!({"serial": "P3RO-TWO", "while_live": true}));
    assert!(ok(&v), "{v}");
    let d = failed_at(&mut admin, "ro1", "P3RO-TWO", "find");
    assert!(
        d["failed"]["text"].as_str().unwrap().contains("2 boards"),
        "{d}"
    );
    assert_eq!(r.bench.flashed(), 0, "nothing was written");
    assert!(d["before"]["export"].is_string(), "the export stays");
    r.hub.stop();
}

#[test]
fn wp_p3ro_a_board_that_does_not_appear_fails_the_find() {
    let (r, mut admin) = ready("none", "P3RO-NONE");
    r.bench.plan.lock().unwrap().after = 0;
    let v = admin.start(json!({"serial": "P3RO-NONE", "while_live": true}));
    assert!(ok(&v), "{v}");
    admin.at_step("ro1", 0, "find");
    std::thread::sleep(Duration::from_millis(300));
    r.advance(Duration::from_secs(16));
    let d = failed_at(&mut admin, "ro1", "P3RO-NONE", "find");
    assert!(
        d["failed"]["text"].as_str().unwrap().contains("15 s"),
        "{d}"
    );
    assert_eq!(r.bench.flashed(), 0);
    r.hub.stop();
}

#[test]
fn wp_p3ro_a_device_that_does_not_come_back_in_60_s_raises_the_alarm() {
    let (r, mut admin) = ready("never", "P3RO-NEVER");
    r.bench
        .plan
        .lock()
        .unwrap()
        .back
        .push(("P3RO-NEVER".into(), Back::Never));
    let v = admin.start(json!({"serial": "P3RO-NEVER", "while_live": true}));
    assert!(ok(&v), "{v}");
    admin.at_step("ro1", 0, "wait");
    std::thread::sleep(Duration::from_millis(300));
    r.advance(Duration::from_secs(61));
    let d = failed_at(&mut admin, "ro1", "P3RO-NEVER", "wait");
    assert!(
        d["failed"]["text"].as_str().unwrap().contains("60 s"),
        "{d}"
    );
    assert_eq!(r.bench.flashed(), 1, "written once, never again");
    r.hub.stop();
}

#[test]
fn wp_p3ro_another_version_or_a_core_voltage_read_back_wrong_fails_the_verify() {
    let (r, mut admin) = ready("version", "P3RO-VER");
    r.bench
        .plan
        .lock()
        .unwrap()
        .back
        .push(("P3RO-VER".into(), Back::As(6, 0x0D)));
    let v = admin.start(json!({"serial": "P3RO-VER", "while_live": true}));
    assert!(ok(&v), "{v}");
    let d = failed_at(&mut admin, "ro1", "P3RO-VER", "verify");
    let text = d["failed"]["text"].as_str().unwrap();
    assert!(text.contains("1.1.6") && text.contains("1.1.7"), "{text}");
    assert_eq!(d["after"]["fw"], "1.1.6");
    r.hub.stop();

    // A 1200 board that comes back with its core voltage at 1.15 V.
    let (r, mut admin) = ready("vreg", "P3RO-VREG");
    r.bench
        .plan
        .lock()
        .unwrap()
        .back
        .push(("P3RO-VREG".into(), Back::As(7, 0x0C)));
    let v = admin.start(json!({"serial": "P3RO-VREG", "while_live": true}));
    assert!(ok(&v), "{v}");
    let d = failed_at(&mut admin, "ro1", "P3RO-VREG", "verify");
    let text = d["failed"]["text"].as_str().unwrap();
    assert!(
        text.contains("needs 1.20 V") && text.contains("1.15 V"),
        "{text}"
    );
    assert_eq!(d["after"]["vreg"]["stored_mv"], 1150);
    // A later rollout of the device that ends ok clears the alarm.
    r.bench.plan.lock().unwrap().back.clear();
    let v = admin.start(json!({"serial": "P3RO-VREG", "while_live": true}));
    assert!(ok(&v), "{v}");
    assert_eq!(admin.ended("ro2")["state"], "ok");
    let start = Instant::now();
    loop {
        let a = admin
            .alarms()
            .into_iter()
            .find(|a| a["code"] == "rollout_failed");
        if a.as_ref().is_some_and(|a| a["state"] == "cleared") {
            break;
        }
        assert!(start.elapsed() < WAIT, "{a:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    r.hub.stop();
}

#[test]
fn wp_p3ro_changed_settings_are_reported_and_nothing_is_recalled() {
    let (r, mut admin) = ready("changed", "P3RO-CHG");
    let serial = "P3RO-CHG";
    // The device runs with a setting the fresh firmware will not have.
    let set = admin.ask(
        json!({"t": "param.set", "serial": serial, "path": "vol.master",
        "value": -33.0}),
    );
    assert!(ok(&set), "{set}");
    let v = admin.start(json!({"serial": serial, "while_live": true}));
    assert!(ok(&v), "{v}");
    let done = admin.ended("ro1");
    assert_eq!(done["state"], "ok", "{done}");
    assert_eq!(done["devices"][0]["after"]["settings"], "changed", "{done}");
    // Nothing recalled: the device keeps what it came back with, and no recall record.
    let now = admin.ask(json!({"t": "param.get", "serial": serial, "path": "vol.master"}));
    assert!(ok(&now), "{now}");
    assert_ne!(now["value"], -33.0, "{now}");
    let events = admin.ask(json!({"t": "event.list", "serial": serial, "limit": 200}));
    assert!(
        !events["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "recall"),
        "{events}"
    );
    r.hub.stop();
}

#[test]
fn wp_p3ro_a_group_rollout_stops_at_the_first_failure() {
    let r = rig("group");
    for s in ["P3RO-GA", "P3RO-GB", "P3RO-GC"] {
        r.bench.plug(s);
    }
    r.bench
        .plan
        .lock()
        .unwrap()
        .back
        .push(("P3RO-GB".into(), Back::As(6, 0x0D)));
    let mut admin = r.admin();
    assert!(ok(
        &admin.put_image("dspi-1.1.7.uf2", &image("1.1.7", Some("1200")))
    ));
    let g = admin.ask(json!({"t": "group.set", "group": {"name": "Rack",
        "members": ["P3RO-GA", "P3RO-GB", "P3RO-GC"]}}));
    assert!(ok(&g), "{g}");
    let gid = g["group"]["id"].as_str().unwrap().to_owned();
    let v = admin.start(json!({"group": gid, "while_live": true}));
    assert!(ok(&v), "{v}");
    assert_eq!(v["rollout"]["target"], format!("group:{gid}"));
    assert_eq!(states_of(&v["rollout"]), ["running", "pending", "pending"]);
    let done = admin.ended("ro1");
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(states_of(&done), ["ok", "failed", "not_tried"], "{done}");
    assert_eq!(done["devices"][1]["failed"]["step"], "verify");
    assert!(done["devices"][2]["steps"].as_array().unwrap().is_empty());
    // C was never touched: one flash each for A and B only.
    let flashed: Vec<String> = r
        .bench
        .flashes
        .lock()
        .unwrap()
        .iter()
        .map(|f| f.0.clone())
        .collect();
    assert_eq!(flashed, ["P3RO-GA", "P3RO-GB"]);
    assert!(!r.bench.entered("P3RO-GC"));
    assert!(admin.rollout_alarm("P3RO-GB").is_some());
    assert!(admin.rollout_alarm("P3RO-GA").is_none());
    // An unknown group.
    let v = admin.start(json!({"group": "g99", "while_live": true}));
    assert_eq!(code(&v), "not_found", "{v}");
    r.hub.stop();
}

#[test]
fn wp_p3ro_cancel_before_the_entry_stops_the_queue_and_after_it_is_conflict() {
    let r = rig("cancel");
    for s in ["P3RO-CA", "P3RO-CB", "P3RO-CC"] {
        r.bench.plug(s);
    }
    let mut admin = r.admin();
    assert!(ok(
        &admin.put_image("dspi-1.1.7.uf2", &image("1.1.7", Some("1200")))
    ));
    let g = admin.ask(json!({"t": "group.set", "group": {"name": "Hall",
        "members": ["P3RO-CA", "P3RO-CB", "P3RO-CC"]}}));
    let gid = g["group"]["id"].as_str().unwrap().to_owned();
    r.bench.hold_flashes();
    let v = admin.start(json!({"group": gid, "while_live": true}));
    assert!(ok(&v), "{v}");
    // A is in its flash: a cancel is refused, the flash goes on.
    admin.at_step("ro1", 0, "flash");
    let c = admin.ask(json!({"t": "fw.rollout.cancel", "rollout": "ro1"}));
    assert_eq!(code(&c), "conflict", "{c}");
    // B's reads are held before its bootloader entry; cancel there.
    r.bench.plan.lock().unwrap().hold_flash = false;
    r.bench.side("P3RO-CB").hold(true);
    r.bench.release_flash();
    admin.at_step("ro1", 1, "check");
    let c = admin.ask(json!({"t": "fw.rollout.cancel", "rollout": "ro1"}));
    assert!(ok(&c), "{c}");
    assert_eq!(c["rollout"]["state"], "cancelled", "{c}");
    assert_eq!(
        states_of(&c["rollout"]),
        ["ok", "cancelled", "not_tried"],
        "{c}"
    );
    r.bench.side("P3RO-CB").hold(false);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(admin.record("ro1")["state"], "cancelled");
    assert!(!r.bench.entered("P3RO-CB"));
    assert_eq!(r.bench.flashed(), 1);
    // A finished rollout is conflict too; an unknown one not_found.
    let c = admin.ask(json!({"t": "fw.rollout.cancel", "rollout": "ro1"}));
    assert_eq!(code(&c), "conflict");
    let c = admin.ask(json!({"t": "fw.rollout.cancel", "rollout": "ro9"}));
    assert_eq!(code(&c), "not_found");
    // And a new rollout may start, with the next id.
    let v = admin.start(json!({"serial": "P3RO-CC", "while_live": true}));
    assert!(ok(&v), "{v}");
    assert_eq!(v["rollout"]["id"], "ro2");
    assert_eq!(admin.ended("ro2")["state"], "ok");
    r.hub.stop();
}

// -- review p3d -- The hub's one rollout slot is free when a rollout's record says it
// ended: a console that starts the next one the moment `fw.rollout.status` (or
// `fw.rollout.changed`) shows the end is not answered `busy`. Before, the slot was freed
// only after the end was written and announced, so a status read that waited on that
// write was answered "ended" with the slot still taken.
#[test]
fn review_p3d_the_slot_is_free_when_the_record_has_ended() {
    let (r, _admin) = ready("slot", "P3D-SLOT");
    let hub = r.hub.handle();
    let me = hub
        .open_console_principal(Role::Admin, "P3D slot test")
        .unwrap();
    let ask = |msg: Value| -> Value {
        serde_json::from_str(&hub.service_request(&me, &msg).expect("answered")).unwrap()
    };
    let start = json!({"t": "fw.rollout.start", "id": 1, "image": "dspi-1.1.7.uf2",
        "serial": "P3D-SLOT", "while_live": true});
    let mut v = ask(start.clone());
    for round in 0..10 {
        assert!(ok(&v), "round {round}: {v}");
        let id = v["rollout"]["id"].as_str().unwrap().to_owned();
        let begun = Instant::now();
        // No pause between the reads: the next start follows the end as closely as a
        // client can.
        loop {
            let st = ask(json!({"t": "fw.rollout.status", "id": 2, "rollout": id}));
            if st["rollouts"][0]["state"] != "running" {
                assert_eq!(st["rollouts"][0]["state"], "ok", "{st}");
                break;
            }
            assert!(begun.elapsed() < WAIT, "rollout {id} did not end");
        }
        v = ask(start.clone());
    }
    assert!(ok(&v), "{v}");
    r.hub.stop();
}

// ── The upload ──────────────────────────────────────────────────────────────

// -- WP P3WRO -- (review p3c N2) a console whose middle chunk failed starts again at
// once: the session's own open upload does not hold it busy for two minutes, while
// another session's upload is left alone and the hub's cap still holds.
#[test]
fn wp_p3wro_seq_0_restarts_the_sessions_own_upload() {
    let r = rig("restart");
    let mut admin = r.admin();
    let img = image("1.1.7", Some("1200"));
    let b64 = data_encoding::BASE64.encode(&img);
    let s0 = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&s0), "{s0}");
    // The console's chunk 1 never arrived (a dropped answer); it starts over at once.
    let s0 = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&s0), "{s0}");
    assert_eq!(s0["received"], 3072, "counted from the new start: {s0}");
    let done = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 1,
        "data": &b64[4096..], "last": true}));
    assert!(ok(&done), "{done}");
    assert_eq!(done["image"]["size"], img.len(), "{done}");
    // Another session's open upload is not touched by this one's restarts, and two
    // open uploads are still the hub's most.
    let mut other = r.admin();
    let o0 = other.ask(json!({"t": "fw.image.put", "name": "o.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&o0), "{o0}");
    let a0 = admin.ask(json!({"t": "fw.image.put", "name": "b.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&a0), "{a0}");
    let a0 = admin.ask(json!({"t": "fw.image.put", "name": "b.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&a0), "a restart does not count against the cap: {a0}");
    let mut third = r.admin();
    let t0 = third.ask(json!({"t": "fw.image.put", "name": "t.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert_eq!(code(&t0), "busy", "{t0}");
    let o1 = other.ask(json!({"t": "fw.image.put", "name": "o.uf2", "seq": 1,
        "data": &b64[4096..], "last": true}));
    assert!(ok(&o1), "{o1}");
    r.hub.stop();
}

#[test]
fn wp_p3ro_the_upload_has_its_caps_and_its_timeout() {
    let r = rig("upload");
    let mut admin = r.admin();
    let img = image("1.1.7", Some("1200"));
    let b64 = data_encoding::BASE64.encode(&img);
    // In order, with the count so far.
    let first = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&first), "{first}");
    assert_eq!(first["received"], 3072);
    // -- WP P3WRO -- (review p3c N2) a seq 0 in the same session starts its upload
    // again: the open one is dropped, so its next chunk names another image.
    let again = admin.ask(
        json!({"t": "fw.image.put", "name": "b.uf2", "seq": 0, "data": "",
        "last": false}),
    );
    assert!(ok(&again), "{again}");
    let stale = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 1,
        "data": &b64[4096..], "last": true}));
    assert_eq!(code(&stale), "bad_request", "{stale}");
    let first = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&first), "{first}");
    let rest = admin.ask(json!({"t": "fw.image.put", "name": "a.uf2", "seq": 1,
        "data": &b64[4096..], "last": true}));
    assert!(ok(&rest), "{rest}");
    assert_eq!(rest["created"], true);
    let i = &rest["image"];
    assert_eq!(i["size"], img.len());
    assert_eq!(i["firmware_version"], "1.1.7");
    assert_eq!(i["added_by"], "Bench console");
    assert_eq!(i["sha256"].as_str().unwrap().len(), 64);
    // The same bytes again: created false; other bytes under the name: conflict.
    let again = admin.put_image("a.uf2", &img);
    assert_eq!(again["created"], false, "{again}");
    let other = admin.put_image("a.uf2", &image("1.1.8", Some("1200")));
    assert_eq!(code(&other), "conflict", "{other}");
    // A gap drops the upload; its next chunk is not_found.
    let s0 = admin.ask(json!({"t": "fw.image.put", "name": "c.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&s0));
    let gap = admin.ask(json!({"t": "fw.image.put", "name": "c.uf2", "seq": 2,
        "data": &b64[4096..8192], "last": false}));
    assert_eq!(code(&gap), "bad_request", "{gap}");
    let gone = admin.ask(json!({"t": "fw.image.put", "name": "c.uf2", "seq": 1,
        "data": &b64[4096..8192], "last": false}));
    assert_eq!(code(&gone), "not_found", "{gone}");
    // A chunk past 49152 characters, data that is not base64, a bad name.
    let big = "A".repeat(49_156);
    let v = admin.ask(
        json!({"t": "fw.image.put", "name": "d.uf2", "seq": 0, "data": big,
        "last": false}),
    );
    assert_eq!(code(&v), "bad_request", "{v}");
    let v = admin.ask(
        json!({"t": "fw.image.put", "name": "d.uf2", "seq": 0, "data": "@@@@",
        "last": false}),
    );
    assert_eq!(code(&v), "bad_request");
    for bad in ["../x.uf2", "x.bin", "index.json", ".uf2", "a b.uf2"] {
        let v = admin.ask(
            json!({"t": "fw.image.put", "name": bad, "seq": 0, "data": "",
            "last": true}),
        );
        assert_eq!(code(&v), "bad_request", "{bad}: {v}");
    }
    // Not firmware at the end: bad_request with the inspector's problem.
    let v = admin.put_image("junk.uf2", b"not a UF2 at all, just bytes");
    assert_eq!(code(&v), "bad_request", "{v}");
    assert!(
        v["msg"].as_str().unwrap().contains("not a firmware file"),
        "{v}"
    );
    // The timeout: 2 minutes from the first chunk.
    let s0 = admin.ask(json!({"t": "fw.image.put", "name": "e.uf2", "seq": 0,
        "data": &b64[..4096], "last": false}));
    assert!(ok(&s0));
    r.advance(Duration::from_secs(121));
    let late = admin.ask(json!({"t": "fw.image.put", "name": "e.uf2", "seq": 1,
        "data": &b64[4096..], "last": true}));
    assert_eq!(code(&late), "not_found", "{late}");
    // Past 8 MiB: bad_request, and dropped.
    let chunk = data_encoding::BASE64.encode(&vec![0u8; 36 * 1024]);
    let mut over = Value::Null;
    for seq in 0..240u32 {
        over = admin.ask(json!({"t": "fw.image.put", "name": "huge.uf2", "seq": seq,
            "data": chunk, "last": false}));
        if !ok(&over) {
            break;
        }
    }
    assert_eq!(code(&over), "bad_request", "{over}");
    assert!(over["msg"].as_str().unwrap().contains("8388608"), "{over}");
    // Four images at most.
    for (n, name) in ["f2.uf2", "f3.uf2", "f4.uf2"].iter().enumerate() {
        let put = admin.put_image(name, &image(&format!("1.2.{n}"), Some("1200")));
        assert!(ok(&put), "{name}: {put}");
    }
    let fifth = admin.put_image("f5.uf2", &image("1.3.0", Some("1200")));
    assert_eq!(code(&fifth), "conflict", "{fifth}");
    let list = admin.ask(json!({"t": "fw.image.list"}));
    let names: Vec<&str> = list["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a.uf2", "f2.uf2", "f3.uf2", "f4.uf2"]);
    // Delete: once, then not_found.
    assert!(ok(
        &admin.ask(json!({"t": "fw.image.delete", "image": "f4.uf2"}))
    ));
    let v = admin.ask(json!({"t": "fw.image.delete", "image": "f4.uf2"}));
    assert_eq!(code(&v), "not_found");
    assert!(!r.store().join("firmware").join("f4.uf2").exists());
    no_base64_in_logs();
    r.hub.stop();
}

// ── The hub's own writes stay three ─────────────────────────────────────────

#[test]
fn wp_p3ro_the_rollout_never_submits_as_the_hubs_own_session() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for f in [
        src.join("rollout.rs"),
        src.join("rollout").join("steps.rs"),
        src.join("rollout").join("images.rs"),
    ] {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for word in [
                "INTERNAL_SESSION_ID",
                "GUARD_SESSION_ID",
                "LOCAL_SESSION_ID",
                ".submit(",
                "REQ_CLEAR_CLIPS",
                "core_vreg_op",
                "core_vreg_manual",
                "core_vreg_cal",
                "VregOp",
                "OVERVOLT",
                "I ACCEPT",
                "flash_to_drive",
            ] {
                assert!(
                    !code.contains(word),
                    "{}:{}: {word} in the rollout's code",
                    f.display(),
                    n + 1
                );
            }
        }
    }
}
