//! WP PGFWUP (PLAN-PG sections 1.8 and 1.9) through the real server: the firmware
//! rollout per generation and board. After the reboot the hub re-opens the device
//! through `paged::open` and verifies the version, `EXT_LIST` and the 1.20 V read-back
//! through whichever envelope the new generation has (the tunnel 0xE010 on the
//! dual-stack port); a group mixes generations; a board without BOOTSEL (the
//! DSPi-Zero, an unknown board id) is excluded with its reason and never sent to its
//! bootloader; our firmware is never given away to an image that may be upstream's
//! (that asks a typed confirmation, which nobody gives in a rollout).
//!
//! The rig is P3RO's (`wp_p3ro_rollout.rs`): fake devices the hub reaches as if over its
//! USB, a fake BOOTSEL scan and writer, the test clock, clients over TLS. The devices
//! are PGFAKE1/2's generation and board fakes, opened with `paged::open` as the hub's
//! bus opens a device; each notes what reaches its wire.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    dead_code
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use dspi_core::{BootDevice, FirmwareGuardBoard, FlashReport};
use dspi_hub::rollout::{BootScan, RolloutConfig, Writer};
use dspi_hub::{
    AccessMode, HistoryConfig, Hub, HubConfig, HubHandle, NetworkConfig, Role, ServiceConfig,
    TestClock, TlsMode,
};
use dspi_link::tls::{self as link_tls, PinMode, TlsPipe};
use dspi_proto::generated::ext as gext;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::PlatformInfo;
use dspi_proto::paged::{EXT_TUNNEL, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{
    Capture, CmdId, DeviceDescriptor, FakeBoard, FakeDevice, FakeOptions, FakeRemote,
    NotificationSource, Transport, TransportCaps, paged,
};
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const WAIT: Duration = Duration::from_secs(30);
/// The DSPi's USB vendor id: a device "on this hub's USB".
const VID: u16 = 0x2E8B;
const T0: u64 = 1_800_000_000;
/// The paged fakes' firmware (`SYS.PLATFORM`: 1.1.7 beta 1); the images say the same.
const V117: &str = "1.1.7-beta1";

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
    let dir = std::env::temp_dir().join("wp-pgfwup").join(format!(
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

// ── The devices: generation and board fakes noting what reaches the wire ────

/// Which fake a device is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Our 1.1.6-generation firmware with PAGEGUARD.
    G1b,
    /// Our dual-stack port onto 1.1.7: paged, the tunnel 0xE010.
    G2b,
    /// The DSPi-Zero on its 1.1.6-generation submodule (board id 1 in byte 7).
    Zero,
    /// A Pico build whose byte 7 names a board this build does not know.
    UnknownBoard,
}

/// What reached one device's wire.
#[derive(Default)]
struct Side {
    entered: AtomicBool,
    /// Paged requests: (id, wValue).
    paged: Mutex<Vec<(CmdId, u16)>>,
    /// Page-0 `REQ_EXTENSION` requests: wValue.
    ext0: Mutex<Vec<u16>>,
}

impl Side {
    fn tunnelled(&self, ext: u16) -> bool {
        self.paged
            .lock()
            .unwrap()
            .iter()
            .any(|(id, v)| *id == EXT_TUNNEL && *v == ext)
    }
}

struct Dev {
    inner: FakeDevice,
    side: Arc<Side>,
    board_id: Option<u8>,
}

impl Dev {
    fn in_(
        &mut self,
        opcode: u8,
        value: u16,
        len: u16,
        upto: bool,
    ) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.side.entered.store(true, Ordering::SeqCst);
            return Ok(vec![1]);
        }
        if opcode == op::REQ_EXTENSION {
            self.side.ext0.lock().unwrap().push(value);
        }
        let mut reply = if upto {
            self.inner.control_in_upto(opcode, value, len)?
        } else {
            self.inner.control_in(opcode, value, len)?
        };
        if opcode == op::REQ_GET_PLATFORM
            && let Some(id) = self.board_id
            && usize::from(len) >= PlatformInfo::FULL_LEN
        {
            reply.resize(7, 0);
            reply.push(id);
        }
        Ok(reply)
    }
}

impl Transport for Dev {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.in_(opcode, value, len, false)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.in_(opcode, value, max_len, true)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        if opcode == op::REQ_EXTENSION {
            self.side.ext0.lock().unwrap().push(value);
        }
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

    fn carries_pages(&self) -> bool {
        self.inner.carries_pages()
    }

    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.side.paged.lock().unwrap().push((id, value));
        self.inner.control_in_paged(id, value, max_len)
    }

    fn control_out_paged(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        self.side.paged.lock().unwrap().push((id, value));
        if id == cmds::SYS_ENTER_BOOTLOADER {
            self.side.entered.store(true, Ordering::SeqCst);
            return Ok(());
        }
        self.inner.control_out_paged(id, value, data)
    }
}

/// A fake of `kind` reporting firmware `1.1.<patch>` (a paged one 1.1.7 beta 1), its
/// build default core voltage `vreg` (0x0D 1.20 V, 0x0C 1.15 V). Violations are let
/// through and counted, so the test reads them (a panic in the hub's device thread
/// would only time out).
fn fake(kind: Kind, serial: &str, patch: u8, vreg: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    capture.platform[2] = 0x10 | (patch & 0x0F);
    capture.platform[5] = patch;
    capture.platform[6] = u8::from(patch >= 7);
    let (generation, board) = match kind {
        Kind::G1b | Kind::UnknownBoard => (
            FakeGeneration::Bofr116 { page_guard: true },
            FakeBoard::Pico,
        ),
        Kind::G2b => (FakeGeneration::Bofr117, FakeBoard::Pico),
        Kind::Zero => (FakeGeneration::Stock116, FakeBoard::Zero),
    };
    let options = FakeOptions {
        generation: Some(generation),
        board,
        surfaces: true,
        analysis: true,
        allow_violation: true,
        ..FakeOptions::default()
    };
    let d = FakeDevice::new(capture, options).with_serial(serial);
    d.remote().vreg_set_build_default(vreg);
    d
}

// ── The fake BOOTSEL scan and writer ────────────────────────────────────────

/// What a device is after its flash: its kind, firmware 1.1.<patch>, core default.
type Back = (Kind, u8, u8);

#[derive(Default)]
struct Bench {
    hub: Mutex<Option<HubHandle>>,
    /// What each device comes back as (default: the dual-stack port at 1.20 V).
    back: Mutex<Vec<(String, Back)>>,
    /// Each attachment by serial, newest last: its side and its fake's remote.
    sides: Mutex<Vec<(String, Arc<Side>, FakeRemote)>>,
    in_boot: Mutex<Option<String>>,
    flashes: Mutex<Vec<(String, FirmwareGuardBoard)>>,
}

impl Bench {
    fn hub(&self) -> HubHandle {
        self.hub.lock().unwrap().clone().unwrap()
    }

    /// Plug a fake of `kind`, opened with `paged::open` as the hub's bus opens one.
    fn plug(&self, serial: &str, (kind, patch, vreg): Back) -> u8 {
        let inner = fake(kind, serial, patch, vreg);
        let remote = inner.remote();
        let side = Arc::new(Side::default());
        self.sides
            .lock()
            .unwrap()
            .push((serial.to_owned(), Arc::clone(&side), remote));
        let board_id = (kind == Kind::UnknownBoard).then_some(9);
        let t = paged::open(Box::new(Dev {
            inner,
            side,
            board_id,
        }))
        .unwrap();
        self.hub().attach(t, VID).unwrap()
    }

    /// The newest attachment of `serial`.
    fn side(&self, serial: &str) -> (Arc<Side>, FakeRemote) {
        let sides = self.sides.lock().unwrap();
        let (_, s, r) = sides.iter().rev().find(|(s, _, _)| s == serial).unwrap();
        (Arc::clone(s), r.clone())
    }

    fn attachments(&self, serial: &str) -> usize {
        self.sides
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _, _)| s == serial)
            .count()
    }

    fn handle_of(&self, serial: &str) -> Option<u8> {
        self.hub()
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == serial && d.state == dspi_hub::DeviceState::Online)
            .map(|d| d.handle)
    }

    fn entered(&self, serial: &str) -> bool {
        self.sides
            .lock()
            .unwrap()
            .iter()
            .any(|(s, side, _)| s == serial && side.entered.load(Ordering::SeqCst))
    }

    /// No fake saw a protocol violation (PLAN-PG section 1.1).
    fn no_violation(&self) {
        for (s, _, r) in self.sides.lock().unwrap().iter() {
            assert_eq!(r.violations(), 0, "{s}: {:?}", r.last_violation());
        }
    }
}

impl BootScan for Bench {
    fn boards(&self) -> Vec<BootDevice> {
        let entered = {
            let sides = self.sides.lock().unwrap();
            sides
                .iter()
                .filter(|(_, side, _)| side.entered.load(Ordering::SeqCst))
                .map(|(s, _, _)| s.clone())
                .next_back()
        };
        if let Some(serial) = entered.or_else(|| self.in_boot.lock().unwrap().clone()) {
            if let Some(h) = self.handle_of(&serial) {
                let _ = self.hub().detach(h);
            }
            *self.in_boot.lock().unwrap() = Some(serial.clone());
            return vec![BootDevice {
                platform: 1,
                serial: Some(serial),
                bus_location: "1-2".into(),
            }];
        }
        Vec::new()
    }
}

impl Writer for Bench {
    fn flash(
        &self,
        _image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        let serial = self.in_boot.lock().unwrap().take().unwrap_or_default();
        self.flashes
            .lock()
            .unwrap()
            .push((serial.clone(), board.clone()));
        for (s, side, _) in self.sides.lock().unwrap().iter() {
            if *s == serial {
                side.entered.store(false, Ordering::SeqCst);
            }
        }
        let back = self
            .back
            .lock()
            .unwrap()
            .iter()
            .find(|(s, _)| *s == serial)
            .map_or((Kind::G2b, 7, 0x0D), |(_, b)| *b);
        self.plug(&serial, back);
        Ok(FlashReport {
            platform: target.platform,
            serial: target.serial.clone(),
            firmware_version: V117.into(),
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
        ..HubConfig::in_process("pgfwup")
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
            "client": {"name": name, "app": "wp_pgfwup", "version": "0"}}));
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

fn states_of(record: &Value) -> Vec<String> {
    record["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["state"].as_str().unwrap().to_owned())
        .collect()
}

/// A rig with the 1.20 V image of 1.1.7 beta 1 (ours: it carries our stamp) stored as
/// `dspi-1.1.7.uf2`.
fn ready(tag: &str) -> (Rig, Ws) {
    let r = rig(tag);
    let mut admin = r.admin();
    let put = admin.put_image("dspi-1.1.7.uf2", &image(V117, Some("1200")));
    assert!(ok(&put), "{put}");
    (r, admin)
}

fn group(admin: &mut Ws, members: &[&str]) -> String {
    let g = admin.ask(json!({"t": "group.set", "group": {"name": "Rack", "members": members}}));
    assert!(ok(&g), "{g}");
    g["group"]["id"].as_str().unwrap().to_owned()
}

// ── Re-open after the reboot, verify through the new envelope ───────────────

#[test]
fn wp_pgfwup_the_rollout_verifies_through_the_new_generations_envelope() {
    let (r, mut admin) = ready("envelope");
    let serial = "PGFWUP-ENV";
    r.bench.plug(serial, (Kind::G1b, 6, 0x0D));
    let (before, _) = r.bench.side(serial);
    let v = admin.start(json!({"serial": serial, "while_live": true}));
    assert!(ok(&v), "{v}");
    // The guard's step names the generation change.
    let guard = v["rollout"]["devices"][0]["steps"][1]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(guard.contains("paged protocol"), "{guard}");
    let done = admin.ended("ro1");
    assert_eq!(done["state"], "ok", "{done}");
    let d = &done["devices"][0];
    // Before: the extension channel on page 0 (0x00), the 1.1.6 generation.
    assert!(before.ext0.lock().unwrap().contains(&gext::EXT_LIST));
    assert!(!before.tunnelled(gext::EXT_LIST));
    // After the reboot the hub opened the device again (a new attachment, the
    // dual-stack port): the version, EXT_LIST and the 1.20 V read-back came through
    // the tunnel 0xE010, and no page-0 request reached it after its probe.
    assert_eq!(r.bench.attachments(serial), 2);
    let (after, _) = r.bench.side(serial);
    assert!(
        after.tunnelled(gext::EXT_LIST),
        "EXT_LIST through the tunnel"
    );
    assert!(
        after.tunnelled(gext::EXT_CORE_VREG),
        "EXT_CORE_VREG through the tunnel"
    );
    assert!(
        after.ext0.lock().unwrap().is_empty(),
        "nothing on 0x00 after the probe"
    );
    assert_eq!(d["after"]["fw"], "1.1.7 beta 1", "{d}");
    assert_eq!(
        d["after"]["ext_list"].as_array().unwrap().len(),
        d["before"]["ext_list"].as_array().unwrap().len()
    );
    assert_eq!(d["after"]["vreg"]["live_mv"], 1200);
    assert_eq!(d["after"]["vreg"]["stored_mv"], 1200);
    let verify = d["steps"][7]["text"].as_str().unwrap();
    assert!(verify.contains("core voltage 1.20 V read back"), "{verify}");
    r.bench.no_violation();
    no_base64_in_logs();
    r.hub.stop();
}

#[test]
fn wp_pgfwup_a_wrong_read_back_through_the_tunnel_fails_the_verify() {
    let (r, mut admin) = ready("readback");
    let serial = "PGFWUP-RB";
    r.bench.plug(serial, (Kind::G1b, 6, 0x0D));
    // It comes back as the port, at 1.15 V.
    r.bench
        .back
        .lock()
        .unwrap()
        .push((serial.into(), (Kind::G2b, 7, 0x0C)));
    let v = admin.start(json!({"serial": serial, "while_live": true}));
    assert!(ok(&v), "{v}");
    let done = admin.ended("ro1");
    assert_eq!(done["state"], "failed", "{done}");
    let d = &done["devices"][0];
    assert_eq!(d["failed"]["step"], "verify", "{d}");
    let text = d["failed"]["text"].as_str().unwrap();
    assert!(text.contains("needs 1.20 V"), "{text}");
    let (after, _) = r.bench.side(serial);
    assert!(after.tunnelled(gext::EXT_CORE_VREG));
    assert!(admin.rollout_alarm(serial).is_some());
    r.bench.no_violation();
    r.hub.stop();
}

// ── A mixed group; boards without BOOTSEL excluded ──────────────────────────

#[test]
fn wp_pgfwup_a_mixed_group_rolls_out_and_excludes_the_zero() {
    let (r, mut admin) = ready("mixed");
    let members = [
        "PGFWUP-Z1",
        "PGFWUP-A",
        "PGFWUP-B",
        "PGFWUP-Z2",
        "PGFWUP-U",
        "PGFWUP-C",
    ];
    r.bench.plug(members[0], (Kind::Zero, 6, 0x0D));
    r.bench.plug(members[1], (Kind::G1b, 6, 0x0D));
    r.bench.plug(members[2], (Kind::G2b, 7, 0x0D));
    r.bench.plug(members[3], (Kind::Zero, 6, 0x0D));
    r.bench.plug(members[4], (Kind::UnknownBoard, 6, 0x0D));
    r.bench.plug(members[5], (Kind::G1b, 6, 0x0D));
    // The 1.1.6-generation member C comes back still on 1.1.6's stack.
    r.bench
        .back
        .lock()
        .unwrap()
        .push((members[5].into(), (Kind::G1b, 7, 0x0D)));
    let gid = group(&mut admin, &members);
    let v = admin.start(json!({"group": gid, "while_live": true}));
    assert!(ok(&v), "{v}");
    // The first member is excluded at the start; the first with BOOTSEL runs.
    assert_eq!(
        states_of(&v["rollout"]),
        [
            "excluded", "running", "pending", "pending", "pending", "pending"
        ],
        "{v}"
    );
    let done = admin.ended("ro1");
    assert_eq!(done["state"], "ok", "{done}");
    assert_eq!(
        states_of(&done),
        ["excluded", "ok", "ok", "excluded", "excluded", "ok"],
        "{done}"
    );
    for i in [0usize, 3, 4] {
        let d = &done["devices"][i];
        assert_eq!(d["failed"]["code"], "no_bootsel", "{d}");
        assert_eq!(d["failed"]["step"], "check", "{d}");
        assert!(d["before"].is_null(), "nothing exported: {d}");
        assert!(!r.bench.entered(members[i]), "{} entered", members[i]);
        assert!(admin.rollout_alarm(members[i]).is_none());
    }
    let zero = done["devices"][0]["failed"]["text"].as_str().unwrap();
    assert!(
        zero.contains("DSPi Zero") && zero.contains("BOOTSEL"),
        "{zero}"
    );
    let unknown = done["devices"][4]["failed"]["text"].as_str().unwrap();
    assert!(unknown.contains("Unknown board"), "{unknown}");
    let flashed: Vec<String> = r
        .bench
        .flashes
        .lock()
        .unwrap()
        .iter()
        .map(|f| f.0.clone())
        .collect();
    assert_eq!(flashed, [members[1], members[2], members[5]]);
    // Each verified through its own generation's envelope.
    let (a, _) = r.bench.side(members[1]);
    assert!(a.tunnelled(gext::EXT_CORE_VREG));
    let (c, _) = r.bench.side(members[5]);
    assert!(c.ext0.lock().unwrap().contains(&gext::EXT_CORE_VREG));
    assert!(!c.tunnelled(gext::EXT_CORE_VREG));
    r.bench.no_violation();
    r.hub.stop();
}

#[test]
fn wp_pgfwup_a_board_without_bootsel_is_refused_a_rollout_with_its_reason() {
    let (r, mut admin) = ready("zero");
    r.bench.plug("PGFWUP-Z", (Kind::Zero, 6, 0x0D));
    r.bench.plug("PGFWUP-U", (Kind::UnknownBoard, 6, 0x0D));
    for serial in ["PGFWUP-Z", "PGFWUP-U"] {
        let v = admin.start(json!({"serial": serial, "while_live": true}));
        assert_eq!(code(&v), "unsupported", "{v}");
        assert_eq!(v["reason"], "no_bootsel", "{v}");
    }
    // A group of none with BOOTSEL.
    let gid = group(&mut admin, &["PGFWUP-Z", "PGFWUP-U"]);
    let v = admin.start(json!({"group": gid, "while_live": true}));
    assert_eq!(code(&v), "unsupported", "{v}");
    assert_eq!(v["reason"], "no_bootsel", "{v}");
    let st = admin.ask(json!({"t": "fw.rollout.status"}));
    assert_eq!(st["rollouts"], json!([]), "a refused start left a record");
    assert!(!r.bench.entered("PGFWUP-Z") && !r.bench.entered("PGFWUP-U"));
    assert!(r.bench.flashes.lock().unwrap().is_empty());
    r.bench.no_violation();
    r.hub.stop();
}

// ── bofr to upstream ────────────────────────────────────────────────────────

#[test]
fn wp_pgfwup_our_firmware_is_never_given_away_by_a_rollout() {
    let r = rig("lineage");
    let serial = "PGFWUP-L";
    // Our firmware at 1.15 V: the voltage rule alone would let an unstamped image go.
    r.bench.plug(serial, (Kind::G1b, 6, 0x0C));
    let mut admin = r.admin();
    let put = admin.put_image("dspi-1.1.7.uf2", &image(V117, None));
    assert!(ok(&put), "{put}");
    let v = admin.start(json!({"serial": serial, "while_live": true}));
    assert_eq!(code(&v), "image_refused", "{v}");
    assert_eq!(v["reason"], "lineage_loss", "{v}");
    assert!(
        v["msg"].as_str().unwrap().contains("extension channel"),
        "{v}"
    );
    assert!(!r.bench.entered(serial));
    r.bench.no_violation();
    r.hub.stop();
}
