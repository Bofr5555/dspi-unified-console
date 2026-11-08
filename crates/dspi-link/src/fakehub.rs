//! A fake DSPi Link hub for tests and for trying the app without hardware
//! (feature `fake-hub`; binary `dspi-link-fakehub`).
//!
//! It serves `/dspi/v1` (WebSocket, sub-protocol `dspi-link-1`) and
//! `/dspi/v1/info` from one simulated device seeded from the DSPi-Link
//! fixtures (`bulk_v32.bin`, `platform.bin`, `status_wvalue9.bin`). Covered:
//! hello, `auth: none` and `pin` (window or fixed PIN), tokens, roles from
//! `policy/commands.json` (viewer/control/admin, `policy.denied`),
//! `device.list`, `device.snapshot`, `poll.subscribe`, locks, CMD with
//! per-device ordering and `max_inflight`, NOTIFY with `origin` attribution
//! for SETs it knows the bulk offset of, RESYNC, close codes 4002/4003, and
//! the bridge's limits (`bridge: true`: 64 B SET, 132 B GET, no 0x0F, no
//! chunked bulk). Not a conformance reference: it is as small as the client
//! tests need.
//!
//! It behaves like the real hubs where that changed what a client sees
//! (ESP32 bridge `hub.c`, `hub_auth.c`, `http_server.c`; Mac `AuthStore`):
//! - WebSocket pings to authenticated sessions every [`HubConfig::ping_interval`],
//!   and close 1001 after [`HubConfig::peer_timeout`] without a client frame,
//!   so a vanished client holds its slot until then;
//! - close 4001 at [`HubConfig::auth_deadline`] from socket open;
//! - a wrong PIN answers `unauthenticated` with no `msg`, never `denied`;
//! - 5 failures from one address in a minute answer `rate_limited` to PINs and
//!   tokens alike for 60 s; 5 failures, or one success, end a PIN window;
//! - [`HubConfig::max_sessions`] answers 503 before the upgrade, and a bridge
//!   answers 400 when the sub-protocol is missing;
//! - a board that goes away ([`FakeHub::set_board_present`]) is reported
//!   `offline`, ends the polls and answers NO_DEVICE, and after
//!   [`HubConfig::board_removal_hold`] is `device.removed`, leaving an empty
//!   `device.list` (bridge spec 7.6). It comes back as `device.changed` within
//!   the hold, and as `device.added` after it;
//! - more than `max_inflight` of a session's commands outstanding answer
//!   RATE_LIMITED (bridge spec 8.3). The fake device answers at once, so
//!   commands are outstanding only while [`FakeHub::hold_replies`] holds them;
//! - polls are granted against one [`HubConfig::poll_budget_bps`] shared by all
//!   sessions (bridge spec 7.7): identical polls count once, a rate may be cut,
//!   and what cannot get 1 Hz answers `busy` with nothing changed.
//!
//! Faults are injected through [`FakeHub`] methods: [`FakeHub::stall`],
//! [`FakeHub::drop_connections`], [`FakeHub::drop_next_replies`],
//! [`FakeHub::hold_replies`] / [`FakeHub::release_replies`],
//! [`FakeHub::duplicate_frames`], [`FakeHub::restart`], [`FakeHub::factory_reset`],
//! [`FakeHub::fail_tokens_with`], [`FakeHub::fail_device_list`] and
//! [`FakeHub::refuse_handshake`]. Timers run on [`HubConfig::clock`], so a test
//! can use a [`ManualClock`](crate::clock::ManualClock) instead of waiting.
//!
//! Not modelled: the bridge's limit of two unauthenticated sessions per
//! address, its 16-entry token table (4002 on eviction), the 20-failure global
//! PIN limit, and close 1011 for a client that stopped reading.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::handshake::server::{Request, Response};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Message, WebSocket};

use crate::clock::{Clock, SystemClock};
use crate::frames::{kind, status, CmdResponse, Dir, Frame, NotifyFrame, PollFrame};

pub const BULK_V32: &[u8] = include_bytes!("../../../tests/fixtures/link/device/bulk_v32.bin");
pub const PLATFORM: &[u8] = include_bytes!("../../../tests/fixtures/link/device/platform.bin");
pub const STATUS_WVALUE9: &[u8] = include_bytes!("../../../tests/fixtures/link/device/status_wvalue9.bin");
const POLICY: &str = include_str!("../../../tests/fixtures/link/policy/commands.json");

/// Firmware request ids the fake device knows.
pub mod req {
    pub const GET_STATUS: u8 = 0x50;
    pub const GET_SERIAL: u8 = 0x7E;
    pub const GET_PLATFORM: u8 = 0x7F;
    pub const GET_ALL_PARAMS: u8 = 0xA0;
    pub const SET_ALL_PARAMS: u8 = 0xA1;
    pub const GET_ALL_PARAMS_CHUNK: u8 = 0xA2;
    pub const SET_ALL_PARAMS_CHUNK: u8 = 0xA3;
    pub const RTA_GET_BANDS_ALL: u8 = 0x0F;
    pub const SET_MASTER_VOLUME: u8 = 0xD2;
    // -- WP CG5 -- firmware config.h:167-174
    pub const RTA_SET_CONFIG: u8 = 0x08;
    pub const RTA_GET_CONFIG: u8 = 0x09;
    pub const RTA_GET_CAPS: u8 = 0x0A;
    pub const RTA_GET_BANDS: u8 = 0x0B;
    pub const RTA_GET_STATUS: u8 = 0x0D;
    pub const RTA_CONTROL: u8 = 0x0E;
    // -- WP CG21 --
    pub const OUTPUT_LATENCY: u8 = 0x03;
    pub const GET_BUFFER_STATS: u8 = 0xB0;
}

/// Bulk offset of the master volume in wire V32 (core `OFF_MASTER_VOLUME`).
pub const OFF_MASTER_VOLUME: u16 = 4700;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    None,
    /// `pin` with a PIN shown on the hub (`pin_source` absent, as a 1.1 Console hub).
    Window(String),
    /// `pin` with the hub's fixed PIN (`pin_source: fixed`, a 1.2 bridge).
    Fixed(String),
}

#[derive(Debug, Clone)]
pub struct HubConfig {
    pub bind: SocketAddr,
    pub hub_id: String,
    pub name: String,
    /// `console` or `bridge`.
    pub kind: String,
    pub proto_minor: u32,
    pub auth: Auth,
    /// Role granted by a correct PIN (a fixed-PIN bridge grants admin).
    pub pin_role: String,
    /// Apply the bridge's command limits (spec 9.2).
    pub bridge_limits: bool,
    pub caps: Vec<String>,
    pub max_inflight: u32,
    pub serial: String,
    pub device_name: String,
    pub image: Vec<u8>,
    /// SET request -> (bulk offset, size): the SETs that emit PARAM_CHANGED.
    pub set_offsets: HashMap<u8, (u16, u16)>,
    /// Requests answered TIMEOUT (the hub gave up on the device).
    pub timeout_requests: HashSet<u8>,
    /// Close 4001 this long after the socket opened without authentication.
    pub auth_deadline: Duration,
    /// WebSocket ping to each authenticated session (both real hubs: 10 s).
    pub ping_interval: Duration,
    /// Close 1001 after this long without a client frame (both real hubs: 30 s).
    /// A client that vanished holds its session slot until then.
    pub peer_timeout: Duration,
    /// Sessions the hub takes; more get HTTP 503 before the upgrade. `None` for
    /// the Mac hub, which has no cap; a bridge has 4, or 8 with PSRAM.
    pub max_sessions: Option<usize>,
    /// Failed `auth.pair` / `auth.token` from one address within
    /// [`Self::rate_limit_window`] before the hub answers `rate_limited`.
    pub rate_limit_failures: usize,
    pub rate_limit_window: Duration,
    /// How long `rate_limited` lasts.
    pub rate_limit_lockout: Duration,
    /// False: the hub starts with no board, and `device.list` is empty.
    pub board_attached: bool,
    /// How long a lost board stays `offline` before `device.removed` (60 s).
    pub board_removal_hold: Duration,
    /// Bytes per second all polls may use together. The hello advertises it.
    /// Mac hub 200000; a bridge at 1 Mbaud 40000.
    pub poll_budget_bps: u32,
    /// The time source of the hub's auth, keepalive, rate-limit, board and poll timers.
    pub clock: Arc<dyn Clock>,
    /// -- WP CG5 -- The device has an analyser. False: every RTA opcode is a
    /// STALL, as on firmware without one.
    pub rta: bool,
    /// -- WP CG21 -- Answer `OUTPUT_LATENCY` and `GET_BUFFER_STATS`.
    pub stats: bool,
    /// -- WP LK1 -- The snapshot's `age_ms`. The ESP32 bridge reports the time
    /// since it last read the blob, which stays current while the DSPi
    /// notifies: hours on a bridge that has been up for hours.
    pub snapshot_age_ms: u64,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:0".parse().unwrap(),
            hub_id: "6f1a2b3c-4d5e-4f60-8a9b-0c1d2e3f4a5b".into(),
            name: "Fake Hub".into(),
            kind: "console".into(),
            proto_minor: 1,
            auth: Auth::None,
            pin_role: "control".into(),
            bridge_limits: false,
            caps: ["cmd", "notify", "poll", "snapshot", "lock", "rename"].iter().map(|s| s.to_string()).collect(),
            max_inflight: 8,
            serial: "0123456789ABCDEF".into(),
            device_name: "Bench".into(),
            image: BULK_V32.to_vec(),
            set_offsets: HashMap::from([(req::SET_MASTER_VOLUME, (OFF_MASTER_VOLUME, 4))]),
            timeout_requests: HashSet::new(),
            auth_deadline: Duration::from_secs(30),
            ping_interval: Duration::from_secs(10),
            peer_timeout: Duration::from_secs(30),
            max_sessions: None,
            rate_limit_failures: 5,
            rate_limit_window: Duration::from_secs(60),
            rate_limit_lockout: Duration::from_secs(60),
            board_attached: true,
            board_removal_hold: Duration::from_secs(60),
            poll_budget_bps: 200_000,
            clock: Arc::new(SystemClock),
            rta: false,         // -- WP CG5 --
            stats: false,       // -- WP CG21 --
            snapshot_age_ms: 0, // -- WP LK1 --
        }
    }
}

impl HubConfig {
    /// The ESP32 bridge's shape (spec 9.2, fixtures `hello_hub_bridge.json`).
    pub fn bridge() -> Self {
        Self {
            hub_id: "0d6c41e2-93b7-4f0a-b5a8-3c2e7f19a4d0".into(),
            name: "DSPi Bridge 3F2A".into(),
            kind: "bridge".into(),
            proto_minor: 2,
            auth: Auth::Fixed("482913".into()),
            pin_role: "admin".into(),
            bridge_limits: true,
            caps: ["cmd", "notify", "poll", "snapshot", "lock", "web", "rename"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            max_inflight: 4,
            max_sessions: Some(4),
            poll_budget_bps: 40_000,
            ..Self::default()
        }
    }
}

/// How [`FakeHub::fail_device_list`] breaks `device.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceListFault {
    /// Answer `err` with this code.
    Error(String),
    /// Never answer.
    Silent,
}

/// One message to a session's socket.
#[derive(Clone)]
enum Out {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
    /// Cut the TCP connection without a close frame.
    Abort,
}

/// What [`FakeHub`]'s fault injection has switched on.
#[derive(Default)]
struct Faults {
    /// Sessions neither read nor write: the network went dark.
    stalled: bool,
    /// Answer the upgrade with this HTTP status.
    refuse_handshake: Option<u16>,
    /// `auth.token` answers `err` with this code.
    token_error: Option<String>,
    device_list: Option<DeviceListFault>,
    device_list_times: u32,
    /// Replies wait in the session's `held` until [`FakeHub::release_replies`].
    hold_replies: bool,
    /// Replies to swallow.
    drop_replies: u32,
    /// Every queued frame and reply goes out twice.
    duplicate: bool,
}

struct SessionRec {
    tx: Sender<Out>,
    role: String,
    authed: bool,
    token: Option<String>,
    link_seq: u16,
    poll_seq: u16,
    polls: HashMap<u8, PollRec>,
    peer: Option<IpAddr>,
    held: Vec<Out>,
}

/// One granted poll of a session.
struct PollRec {
    /// `bRequest`, `wValue` and `wLength` as bytes: what makes polls identical.
    spec: Vec<u8>,
    hz: u32,
    every: Duration,
    next: Instant,
}

/// The fastest rate a poll may have (`hello.limits.poll_max_hz`).
const POLL_MAX_HZ: u32 = 20;
/// Bytes a poll costs on the link beyond its answer: request header, CRC and
/// response overhead (bridge `poll_cost_bytes`).
const POLL_OVERHEAD_BYTES: u64 = 18;

fn poll_cost(spec: &[u8]) -> u64 {
    POLL_OVERHEAD_BYTES + u64::from(u16::from_le_bytes([spec[3], spec[4]]))
}

/// Where the simulated board is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Board {
    Online,
    /// Lost at this time; handle kept until [`HubConfig::board_removal_hold`].
    Offline(Instant),
    Absent,
}

/// Auth failures of one address (spec 6.2).
#[derive(Default)]
struct FailureRecord {
    failures: Vec<Instant>,
    locked_until: Option<Instant>,
}

/// -- WP CG21 -- The output rings: what a Stats tick reads.
#[derive(Debug, Clone, Copy)]
struct FakeRing {
    requested: u8,
    target_frames: u16,
    pending: bool,
}

impl Default for FakeRing {
    fn default() -> Self {
        Self { requested: 3, target_frames: 96, pending: false }
    }
}

impl FakeRing {
    /// `OutputLatencyStatus` (config.h), 12 bytes.
    fn status(&self) -> Vec<u8> {
        let mut s = vec![0u8; 12];
        s[0] = self.requested;
        s[1] = self.requested;
        s[2..4].copy_from_slice(&2000u16.to_le_bytes());
        s[4..6].copy_from_slice(&self.target_frames.to_le_bytes());
        s[6..8].copy_from_slice(&48u16.to_le_bytes());
        s[8] = u8::from(self.pending);
        s[9] = 7;
        s
    }

    /// `BufferStatsPacket`, 44 bytes: one output at 96 of 768 frames.
    fn buffer_stats() -> Vec<u8> {
        let mut b = vec![0u8; 44];
        b[0] = 1;
        b[4 + 3..4 + 6].copy_from_slice(&[12, 6, 25]);
        b[4 + 6..4 + 8].copy_from_slice(&1250u16.to_le_bytes());
        b
    }
}

struct State {
    cfg: HubConfig,
    image: Vec<u8>,
    rta: FakeRta,   // -- WP CG5 --
    ring: FakeRing, // -- WP CG21 --
    fw_seq: u8,
    next_session: u16,
    sessions: HashMap<u16, SessionRec>,
    tokens: HashMap<String, String>,
    lock: Option<(u16, Instant)>,
    log: Vec<(u16, u8, u8, u16)>,
    classes: HashMap<(u8, u8), String>,
    faults: Faults,
    failures: HashMap<Option<IpAddr>, FailureRecord>,
    /// A PIN window is open (spec 6.2: 5 failures or one success end it).
    window_open: bool,
    window_failures: u32,
    board: Board,
    pings_sent: u64,
    pongs_received: u64,
    /// Every token presented to `auth.token`.
    tokens_seen: Vec<String>,
}

/// A running fake hub; stops when dropped.
pub struct FakeHub {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// `(bRequest, dir)` -> class from `policy/commands.json` (spec 9.3).
fn load_policy() -> HashMap<(u8, u8), String> {
    let v: Value = serde_json::from_str(POLICY).unwrap_or_default();
    let mut out = HashMap::new();
    for c in v["commands"].as_array().into_iter().flatten() {
        let Some(code) = c["code"].as_u64() else { continue };
        let class = c["class"].as_str().unwrap_or("config");
        for (d, name) in [(0u8, "set"), (1u8, "get")] {
            if c["dirs"].as_array().is_some_and(|ds| ds.iter().any(|x| x == name)) {
                let k = c["classes"][name].as_str().unwrap_or(class);
                out.insert((code as u8, d), k.to_owned());
            }
        }
    }
    out
}

fn allowed(role: &str, class: &str) -> bool {
    match role {
        "admin" => true,
        "control" => class != "config",
        _ => class == "read",
    }
}

impl State {
    fn class(&self, b_request: u8, dir: u8) -> &str {
        self.classes.get(&(b_request, dir)).map(String::as_str).unwrap_or("config")
    }

    fn denied(&self, role: &str) -> Vec<[u8; 2]> {
        let mut d: Vec<[u8; 2]> =
            self.classes.iter().filter(|(_, c)| !allowed(role, c)).map(|((r, dir), _)| [*r, *dir]).collect();
        d.sort();
        d
    }

    fn now(&self) -> Instant {
        self.cfg.clock.now()
    }

    fn peer_of(&self, sid: u16) -> Option<IpAddr> {
        self.sessions.get(&sid).and_then(|r| r.peer)
    }

    /// The address is in its `rate_limited` minute.
    fn rate_limited(&mut self, peer: Option<IpAddr>) -> bool {
        let now = self.now();
        let Some(r) = self.failures.get_mut(&peer) else { return false };
        match r.locked_until {
            Some(until) if now < until => true,
            Some(_) => {
                *r = FailureRecord::default();
                false
            }
            None => false,
        }
    }

    fn auth_failed(&mut self, peer: Option<IpAddr>) {
        let now = self.now();
        let (limit, window, lockout) =
            (self.cfg.rate_limit_failures, self.cfg.rate_limit_window, self.cfg.rate_limit_lockout);
        let r = self.failures.entry(peer).or_default();
        r.failures.retain(|t| now.saturating_duration_since(*t) < window);
        r.failures.push(now);
        if r.failures.len() >= limit {
            r.locked_until = Some(now + lockout);
        }
    }

    fn device_json(&self) -> Value {
        let p = PLATFORM;
        json!({
            "handle": 0, "serial": self.cfg.serial, "name": self.cfg.device_name,
            "platform": p[0], "fw": format!("{}.{}.{}", p[1], p[4], p[5]), "fw_beta": p[6], "outputs": p[3],
            "wire_version": self.image[0], "state": if self.board == Board::Online { "online" } else { "offline" },
            "link": if self.cfg.kind == "bridge" { "uart" } else { "usb" },
            "locked_by": self.lock.map(|l| l.0),
        })
    }

    /// Replies to this session's commands not delivered yet.
    fn outstanding(&self, sid: u16) -> usize {
        self.sessions.get(&sid).map_or(0, |r| r.held.iter().filter(|o| matches!(o, Out::Binary(_))).count())
    }

    fn set_board(&mut self, present: bool) {
        match (present, self.board) {
            (false, Board::Online) => {
                self.board = Board::Offline(self.now());
                // Spec 7.6: a subscription ends when the device goes offline
                for r in self.sessions.values_mut() {
                    r.polls.clear();
                }
                let changed = json!({"t": "device.changed", "device": self.device_json()}).to_string();
                self.broadcast(changed);
            }
            (true, Board::Offline(_)) => {
                self.board = Board::Online;
                let changed = json!({"t": "device.changed", "device": self.device_json()}).to_string();
                self.broadcast(changed);
            }
            (true, Board::Absent) => {
                self.board = Board::Online;
                let added = json!({"t": "device.added", "device": self.device_json()}).to_string();
                self.broadcast(added);
            }
            _ => {}
        }
    }

    /// A lost board's handle runs out.
    fn board_tick(&mut self) {
        if let Board::Offline(since) = self.board {
            if self.now().saturating_duration_since(since) >= self.cfg.board_removal_hold {
                self.board = Board::Absent;
                self.lock = None;
                let removed = json!({"t": "device.removed", "handle": 0, "serial": self.cfg.serial}).to_string();
                self.broadcast(removed);
            }
        }
    }

    /// The rates for `requests` (`slot`, spec, wanted Hz) of session `sid`, or
    /// `None` if one cannot get 1 Hz (bridge `grant_rates`): all sessions share
    /// one budget, a poll identical to a running one costs only the rate it adds,
    /// and the slots being replaced no longer count.
    fn grant_polls(&self, sid: u16, requests: &[(u8, Vec<u8>, u32)]) -> Option<Vec<u32>> {
        let mut rates: HashMap<&[u8], u32> = HashMap::new();
        for (id, r) in self.sessions.iter().filter(|(_, r)| r.authed) {
            for (slot, p) in &r.polls {
                if !(*id == sid && requests.iter().any(|q| q.0 == *slot)) {
                    let rate = rates.entry(&p.spec).or_default();
                    *rate = (*rate).max(p.hz);
                }
            }
        }
        let budget = u64::from(self.cfg.poll_budget_bps);
        let mut used: u64 = rates.iter().map(|(spec, hz)| poll_cost(spec) * u64::from(*hz)).sum();
        let mut granted = Vec::with_capacity(requests.len());
        for (_, spec, wanted) in requests {
            let cost = poll_cost(spec);
            let running = rates.get(spec.as_slice()).copied().unwrap_or(0);
            let mut hz = (*wanted).min(POLL_MAX_HZ);
            if hz > running {
                let affordable = budget.saturating_sub(used) / cost;
                hz = hz.min(u32::try_from(u64::from(running) + affordable).unwrap_or(u32::MAX));
            }
            if hz < 1 {
                return None;
            }
            if hz > running {
                used += cost * u64::from(hz - running);
                rates.insert(spec, hz);
            }
            granted.push(hz);
        }
        Some(granted)
    }

    fn broadcast(&mut self, text: String) {
        for s in self.sessions.values() {
            if s.authed {
                let _ = s.tx.send(Out::Text(text.clone()));
            }
        }
    }

    /// Relay a v2 packet to every authenticated session (spec 8.3).
    fn notify(&mut self, mut packet: Vec<u8>, origin: u16) {
        if self.board != Board::Online {
            return; // Relayed only while a device is present (bridge spec 7.6)
        }
        packet[3] = self.fw_seq;
        self.fw_seq = self.fw_seq.wrapping_add(1);
        for s in self.sessions.values_mut() {
            if s.authed {
                let f = Frame::Notify(NotifyFrame { seq: s.link_seq, handle: 0, origin, packet: packet.clone() });
                s.link_seq = s.link_seq.wrapping_add(1);
                let _ = s.tx.send(Out::Binary(f.encode()));
            }
        }
    }

    fn param_changed(&mut self, off: u16, value: &[u8], source: u8, origin: u16) {
        let mut p = vec![0x02, 0x02, 0, 0];
        p.extend_from_slice(&off.to_le_bytes());
        p.extend_from_slice(&(value.len() as u16).to_le_bytes());
        p.extend_from_slice(&[source, 0, 0, 0]);
        p.extend_from_slice(value);
        self.notify(p, origin);
    }

    /// Run one vendor request on the simulated device.
    fn device(
        &mut self,
        sid: u16,
        dir: Dir,
        b_request: u8,
        w_value: u16,
        w_length: u16,
        payload: &[u8],
    ) -> (u8, Vec<u8>) {
        if self.cfg.timeout_requests.contains(&b_request) {
            return (status::TIMEOUT, Vec::new());
        }
        match (dir, b_request) {
            // -- WP CG5 --
            (_, 0x08..=0x0F) if self.cfg.rta => self.rta_device(sid, dir, b_request, w_value, payload),
            // -- WP CG21 --
            (Dir::Get, req::OUTPUT_LATENCY) if self.cfg.stats => (status::OK, self.ring.status()),
            (Dir::Set, req::OUTPUT_LATENCY) if self.cfg.stats => {
                self.ring.requested = payload.first().copied().unwrap_or(1);
                self.ring.target_frames = u16::from(self.ring.requested) * 64;
                (status::OK, Vec::new())
            }
            (Dir::Get, req::GET_BUFFER_STATS) if self.cfg.stats => (status::OK, FakeRing::buffer_stats()),
            (Dir::Get, req::GET_PLATFORM) => (status::OK, PLATFORM.to_vec()),
            (Dir::Get, req::GET_SERIAL) => (status::OK, self.cfg.serial.as_bytes().to_vec()),
            (Dir::Get, req::GET_STATUS) => (status::OK, STATUS_WVALUE9.to_vec()),
            (Dir::Get, req::GET_ALL_PARAMS) => (status::OK, self.image.clone()),
            (Dir::Get, req::GET_ALL_PARAMS_CHUNK) => {
                let o = (w_value as usize).min(self.image.len());
                let e = (o + w_length as usize).min(self.image.len());
                if o >= self.image.len() {
                    (status::ERROR, Vec::new())
                } else {
                    (status::OK, self.image[o..e].to_vec())
                }
            }
            (Dir::Set, req::SET_ALL_PARAMS) if payload.len() == self.image.len() => {
                self.image = payload.to_vec();
                self.notify(vec![0x02, 0x03, 0, 0, 2, 0, 0, 0], sid);
                (status::OK, Vec::new())
            }
            (Dir::Set, req::SET_ALL_PARAMS_CHUNK) => {
                let o = w_value as usize;
                if o + payload.len() <= self.image.len() {
                    self.image[o..o + payload.len()].copy_from_slice(payload);
                    if o + payload.len() == self.image.len() {
                        self.notify(vec![0x02, 0x03, 0, 0, 2, 0, 0, 0], sid);
                    }
                    (status::OK, Vec::new())
                } else {
                    (status::ERROR, Vec::new())
                }
            }
            (Dir::Set, r) => {
                if let Some(&(off, size)) = self.cfg.set_offsets.get(&r) {
                    if payload.len() == size as usize && (off as usize + payload.len()) <= self.image.len() {
                        let o = off as usize;
                        self.image[o..o + payload.len()].copy_from_slice(payload);
                        self.param_changed(off, payload, 1, sid);
                    }
                }
                (status::OK, Vec::new())
            }
            // An opcode the fake firmware does not have: a STALL
            (Dir::Get, _) => (status::ERROR, Vec::new()),
        }
    }

    fn command(&mut self, sid: u16, f: crate::frames::CmdRequest) -> CmdResponse {
        let reply = |status: u8, payload: Vec<u8>| CmdResponse { tag: f.tag, status, payload };
        let role = self.sessions.get(&sid).map(|s| s.role.clone()).unwrap_or_default();
        // In the order of bridge spec 8.3
        if f.handle != 0 || self.board != Board::Online {
            return reply(status::NO_DEVICE, Vec::new());
        }
        if f.payload.len() > 8192 || (f.dir == Dir::Get && f.w_length as usize > 8192) {
            return reply(status::TOO_LARGE, Vec::new());
        }
        if !allowed(&role, self.class(f.b_request, f.dir as u8)) {
            return reply(status::DENIED, Vec::new());
        }
        if let Some((holder, until)) = self.lock {
            if Instant::now() > until {
                self.lock = None;
            } else if holder != sid {
                return reply(status::LOCKED, Vec::new());
            }
        }
        if self.outstanding(sid) >= self.cfg.max_inflight as usize {
            return reply(status::RATE_LIMITED, Vec::new());
        }
        let bulk = matches!(f.b_request, req::GET_ALL_PARAMS | req::SET_ALL_PARAMS);
        if self.cfg.bridge_limits {
            if matches!(f.b_request, req::GET_ALL_PARAMS_CHUNK | req::SET_ALL_PARAMS_CHUNK) {
                return reply(status::BLOCKED, Vec::new());
            }
            if f.b_request == req::RTA_GET_BANDS_ALL {
                return reply(status::ERROR, Vec::new());
            }
            if f.dir == Dir::Set && !bulk && f.payload.len() > 64 {
                return reply(status::OVERSIZE, Vec::new());
            }
        }
        self.log.push((sid, f.dir as u8, f.b_request, f.w_value));
        let (st, mut data) = self.device(sid, f.dir, f.b_request, f.w_value, f.w_length, &f.payload);
        let cap = if self.cfg.bridge_limits && !bulk { 132 } else { usize::MAX };
        data.truncate((f.w_length as usize).min(cap));
        if f.dir == Dir::Set {
            data.clear();
        }
        reply(st, data)
    }
}

// -- WP CG5 --
// The simulated analyser (`HubConfig::rta`). Its reply layouts are the firmware's
// (DSPi `rta.h`:60-129, `RtaConfig`, `RtaCaps`, `RtaBandFrame`, `RtaStatus`). It has
// no audio: every channel reads the same flat frame. No bins (0x0C): a bin frame
// is only published after a transform, so that opcode STALLs as before the first.
// A SET applies at once, where the firmware does so from its main loop.

const RTA_VERSION: u8 = 3; // rta.h:28
const RTA_INPUTS: u32 = 2;
const RTA_OUTPUTS: u32 = 4;
const RTA_BANDS: usize = 37; // rta.h, RTA_MAX_BANDS
const RTA_LEVEL: u8 = 200; // -- WP QD8b --
const RTA_CENTRES: [u16; RTA_BANDS] = [
    16, 20, 25, 31, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000, 1250, 1600, 2000, 2500,
    3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000, 25000, 31500, 40000, 50000, 63000,
];

struct FakeRta {
    config: [u8; 12],
    running: bool,
    /// -- WP QD8b -- What each band reads, as the frame's `avg` byte.
    level: u8,
}

impl Default for FakeRta {
    fn default() -> Self {
        // Output tap, channel 0, 1024 points, 300 ms, 12 dB/s.
        let mut config = [RTA_VERSION, 1, 1, 0, 10, 0, 0, 0, 12, 0, 0, 0];
        config[6..8].copy_from_slice(&300u16.to_le_bytes());
        Self { config, running: false, level: RTA_LEVEL }
    }
}

impl FakeRta {
    fn mask(&self) -> u16 {
        u16::from_le_bytes([self.config[2], self.config[3]])
    }

    /// `tap_valid_mask` (rta.c), for tap byte `tap`.
    fn tap_mask(tap: u8) -> u16 {
        let width = if tap == 0 { RTA_INPUTS } else { RTA_OUTPUTS };
        ((1u32 << width) - 1) as u16
    }

    /// `config_valid` and `apply_config` (rta.c:370-395): the clamps too.
    fn apply(&mut self, p: &[u8]) -> bool {
        let Ok(mut c) = <[u8; 12]>::try_from(p) else {
            return false;
        };
        let valid = c[0] == RTA_VERSION && c[1] <= 1 && (8..=10).contains(&c[4]);
        let mask = u16::from_le_bytes([c[2], c[3]]) & Self::tap_mask(c[1]);
        if !valid || mask == 0 {
            return false;
        }
        c[2..4].copy_from_slice(&mask.to_le_bytes());
        let avg = u16::from_le_bytes([c[6], c[7]]).min(10000);
        c[6..8].copy_from_slice(&avg.to_le_bytes());
        c[8] = c[8].min(100);
        (c[5], c[10], c[11]) = (0, 0, 0);
        self.config = c;
        true
    }

    /// `RtaCaps`, 16 bytes (rta.h:73-87).
    fn caps() -> Vec<u8> {
        let mut c = vec![RTA_VERSION, RTA_INPUTS as u8, RTA_OUTPUTS as u8, 8, 10, 10, 14, RTA_BANDS as u8, 243, 120];
        c.extend_from_slice(&5000u16.to_le_bytes());
        c.extend_from_slice(&529u16.to_le_bytes());
        c.extend_from_slice(&70u16.to_le_bytes());
        c
    }

    /// `RtaBandFrame`, 82 bytes (rta.h:90-99): 34 bands at the FFT's table.
    fn band_frame(channel: u8, level: u8) -> Vec<u8> {
        let mut f = vec![RTA_VERSION, channel, 1, 34];
        f.extend_from_slice(&10u16.to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&[level; RTA_BANDS]);
        f.extend_from_slice(&[243; RTA_BANDS]);
        f
    }

    /// `RtaStatus`, 24 bytes (rta.h:113-129).
    fn status(&self) -> Vec<u8> {
        let live = if self.running { self.mask() } else { 0 };
        let mut s = vec![RTA_VERSION, u8::from(self.running), self.config[1], 0xFF, 0, live.count_ones() as u8];
        s.extend_from_slice(&live.to_le_bytes());
        s.extend_from_slice(&90u16.to_le_bytes()); // frames_per_s
        s.extend_from_slice(&[0; 6]); // busy, last frame, idle
        s.extend_from_slice(&48_000u32.to_le_bytes());
        s.extend_from_slice(&[0, 0, 0, 0]); // first_band, reserved, bass busy
        s
    }
}

impl State {
    /// The analyser's vendor commands (firmware `vendor_commands.c`:1948, 4499-4600).
    fn rta_device(&mut self, sid: u16, dir: Dir, b_request: u8, w_value: u16, payload: &[u8]) -> (u8, Vec<u8>) {
        match (dir, b_request) {
            (Dir::Set, req::RTA_SET_CONFIG) => {
                if !self.rta.apply(payload) {
                    return (status::ERROR, Vec::new());
                }
                // notify.c, NOTIFY_EVT_RTA_CONFIG: header, the applied config, source.
                let mut p = vec![0x02, 0x0D, 0, 0];
                p.extend_from_slice(&self.rta.config);
                p.push(1);
                self.notify(p, sid);
                (status::OK, Vec::new())
            }
            (Dir::Get, req::RTA_GET_CONFIG) => (status::OK, self.rta.config.to_vec()),
            (Dir::Get, req::RTA_GET_CAPS) if w_value == 0 => (status::OK, FakeRta::caps()),
            // Chunks of 32 centres, from 1 (rta_get_caps_centres, rta.c:436-448).
            (Dir::Get, req::RTA_GET_CAPS) => {
                let first = (w_value as usize - 1).saturating_mul(32);
                if first >= RTA_BANDS {
                    return (status::ERROR, Vec::new());
                }
                (status::OK, RTA_CENTRES.iter().skip(first).take(32).flat_map(|c| c.to_le_bytes()).collect())
            }
            // A band read starts the analyser (`rta_note_read`).
            (Dir::Get, req::RTA_GET_BANDS) => {
                let width = if self.rta.config[1] == 0 { RTA_INPUTS } else { RTA_OUTPUTS };
                if u32::from(w_value) >= width {
                    return (status::ERROR, Vec::new());
                }
                self.rta.running = true;
                (status::OK, FakeRta::band_frame(w_value as u8, self.rta.level))
            }
            (Dir::Get, req::RTA_GET_BANDS_ALL) => {
                self.rta.running = true;
                let mask = self.rta.mask();
                let level = self.rta.level;
                (
                    status::OK,
                    (0..16u8).filter(|c| mask >> c & 1 != 0).flat_map(|c| FakeRta::band_frame(c, level)).collect(),
                )
            }
            (Dir::Get, req::RTA_GET_STATUS) => (status::OK, self.rta.status()),
            // The configuration survives STOP.
            (Dir::Get, req::RTA_CONTROL) if w_value <= 2 => {
                match w_value {
                    0 => self.rta.running = false,
                    1 => self.rta.running = true,
                    _ => {}
                }
                (status::OK, vec![1])
            }
            _ => (status::ERROR, Vec::new()),
        }
    }
}

impl FakeHub {
    pub fn start(cfg: HubConfig) -> io::Result<FakeHub> {
        let listener = TcpListener::bind(cfg.bind)?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let board_attached = cfg.board_attached;
        let state = Arc::new(Mutex::new(State {
            image: cfg.image.clone(),
            rta: FakeRta::default(),   // -- WP CG5 --
            ring: FakeRing::default(), // -- WP CG21 --
            cfg,
            faults: Faults::default(),
            failures: HashMap::new(),
            window_open: true,
            window_failures: 0,
            board: if board_attached { Board::Online } else { Board::Absent },
            pings_sent: 0,
            pongs_received: 0,
            tokens_seen: Vec::new(),
            fw_seq: 0,
            next_session: 12,
            sessions: HashMap::new(),
            tokens: HashMap::new(),
            lock: None,
            log: Vec::new(),
            classes: load_policy(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (Arc::clone(&state), Arc::clone(&stop));
        thread::Builder::new().name("fakehub-accept".into()).spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                lock(&st).board_tick();
                match listener.accept() {
                    Ok((tcp, _)) => {
                        let (st, sp) = (Arc::clone(&st), Arc::clone(&sp));
                        let _ = thread::Builder::new().name("fakehub-conn".into()).spawn(move || serve(tcp, st, sp));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        })?;
        Ok(FakeHub { addr, state, stop })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("ws://{}/dspi/v1", self.addr)
    }

    /// The simulated device's current bulk image.
    pub fn image(&self) -> Vec<u8> {
        lock(&self.state).image.clone()
    }

    /// Every command the device ran: `(session, dir, bRequest, wValue)`.
    pub fn log(&self) -> Vec<(u16, u8, u8, u16)> {
        lock(&self.state).log.clone()
    }

    /// Authenticated session ids.
    pub fn sessions(&self) -> Vec<u16> {
        let mut v: Vec<u16> = lock(&self.state).sessions.iter().filter(|(_, s)| s.authed).map(|(k, _)| *k).collect();
        v.sort();
        v
    }

    /// A change made elsewhere (a knob, another console): patch the image and
    /// notify every session with `source` and `origin`.
    pub fn external_change(&self, off: u16, value: &[u8], source: u8, origin: u16) {
        let mut s = lock(&self.state);
        let o = off as usize;
        s.image[o..o + value.len()].copy_from_slice(value);
        s.param_changed(off, value, source, origin);
    }

    /// -- WP CG21 -- The ring target changed elsewhere (another console), which
    /// the firmware does not notify. `pending`: the change has yet to apply.
    pub fn set_ring_target(&self, frames: u16, pending: bool) {
        let mut s = lock(&self.state);
        s.ring.target_frames = frames;
        s.ring.pending = pending;
    }

    /// -- WP CG5 -- The analyser configuration the device holds (12 bytes).
    pub fn rta_config(&self) -> [u8; 12] {
        lock(&self.state).rta.config
    }

    /// Send RESYNC (reason 2, cache rebuilt) to every session.
    pub fn resync(&self) {
        let s = lock(&self.state);
        for r in s.sessions.values() {
            let _ = r.tx.send(Out::Binary(Frame::Resync { handle: 0, reason: 2 }.encode()));
        }
    }

    /// Skip one Link NOTIFY sequence number for every session (a dropped event).
    pub fn drop_one_notification(&self) {
        let mut s = lock(&self.state);
        for r in s.sessions.values_mut() {
            r.link_seq = r.link_seq.wrapping_add(1);
        }
    }

    /// Hold the device lock as session `sid` (another console).
    pub fn hold_lock(&self, sid: u16, for_ms: u64) {
        lock(&self.state).lock = Some((sid, Instant::now() + Duration::from_millis(for_ms)));
    }

    pub fn release_lock(&self) {
        lock(&self.state).lock = None;
    }

    pub fn lock_holder(&self) -> Option<u16> {
        lock(&self.state).lock.map(|l| l.0)
    }

    /// Revoke every token: their sessions close with 4002 (spec 6.2).
    pub fn revoke_all(&self) {
        let mut s = lock(&self.state);
        s.tokens.clear();
        for r in s.sessions.values() {
            if r.token.is_some() {
                let _ = r.tx.send(Out::Close(4002));
            }
        }
    }

    /// Close every session with `code` (4003 = hub shutting down).
    pub fn close_all(&self, code: u16) {
        let s = lock(&self.state);
        for r in s.sessions.values() {
            let _ = r.tx.send(Out::Close(code));
        }
    }

    // ── Fault injection ───────────────────────────────────────────────

    /// The network goes dark for every session, or comes back: nothing is read
    /// or written, no pings, no timers. TCP stays up (a half-open connection).
    pub fn stall(&self, on: bool) {
        lock(&self.state).faults.stalled = on;
    }

    /// Cut every session's TCP connection without a close frame.
    pub fn drop_connections(&self) {
        let s = lock(&self.state);
        for r in s.sessions.values() {
            let _ = r.tx.send(Out::Abort);
        }
    }

    /// Swallow the next `n` replies (JSON and CMD responses).
    pub fn drop_next_replies(&self, n: u32) {
        lock(&self.state).faults.drop_replies = n;
    }

    /// Keep replies back from now on, until [`Self::release_replies`].
    pub fn hold_replies(&self) {
        lock(&self.state).faults.hold_replies = true;
    }

    /// Deliver what [`Self::hold_replies`] kept back, late, and stop holding.
    pub fn release_replies(&self) {
        let mut s = lock(&self.state);
        s.faults.hold_replies = false;
        for r in s.sessions.values_mut() {
            for o in r.held.drain(..) {
                let _ = r.tx.send(o);
            }
        }
    }

    /// Send every frame and reply twice.
    pub fn duplicate_frames(&self, on: bool) {
        lock(&self.state).faults.duplicate = on;
    }

    /// A reboot: every connection drops with no close frame, locks and rate
    /// limits are gone, and the hub id and tokens survive (the bridge keeps them
    /// in NVS). The port stays bound, so a client can reconnect at once.
    pub fn restart(&self) {
        self.drop_connections();
        let mut s = lock(&self.state);
        s.lock = None;
        s.failures.clear();
        s.faults = Faults::default();
    }

    /// A factory reset, or another hub taking over this address: every
    /// connection drops, `hub_id` is the new id and all tokens are gone.
    pub fn factory_reset(&self, hub_id: &str) {
        self.restart();
        let mut s = lock(&self.state);
        s.cfg.hub_id = hub_id.to_owned();
        s.tokens.clear();
        s.window_open = true;
        s.window_failures = 0;
    }

    /// Answer every `auth.token` with `err` `code` (`None` ends the fault):
    /// `unauthenticated`, `denied`, `rate_limited`, `internal`, ...
    pub fn fail_tokens_with(&self, code: Option<&str>) {
        lock(&self.state).faults.token_error = code.map(str::to_owned);
    }

    /// Break the next `times` `device.list` requests.
    pub fn fail_device_list(&self, fault: DeviceListFault, times: u32) {
        let mut s = lock(&self.state);
        s.faults.device_list = Some(fault);
        s.faults.device_list_times = times;
    }

    /// Answer the WebSocket upgrade with this HTTP status (`None` ends the fault).
    pub fn refuse_handshake(&self, status: Option<u16>) {
        lock(&self.state).faults.refuse_handshake = status;
    }

    /// Lose or attach the board, with the events a real hub sends. Lost: every
    /// session hears `device.changed` with `state: offline`, polls end and
    /// commands answer NO_DEVICE; [`HubConfig::board_removal_hold`] later comes
    /// `device.removed`. Back: `device.changed`, or `device.added` after the
    /// removal. Pairing works throughout (`hub.c`).
    pub fn set_board_present(&self, present: bool) {
        lock(&self.state).set_board(present);
    }

    /// -- WP QD8b -- What the analyser's bands read from now on. A client's snapshot
    /// version moves only when the picture does, and a constant signal draws one
    /// picture: this is how a test sees that a client still polls.
    pub fn set_rta_level(&self, level: u8) {
        lock(&self.state).rta.level = level;
    }

    /// Open a new PIN window (Mac hub: the user presses the button again).
    pub fn reopen_window(&self) {
        let mut s = lock(&self.state);
        s.window_open = true;
        s.window_failures = 0;
    }

    /// Sessions holding a slot, authenticated or not.
    pub fn slots_in_use(&self) -> usize {
        lock(&self.state).sessions.len()
    }

    pub fn pings_sent(&self) -> u64 {
        lock(&self.state).pings_sent
    }

    pub fn pongs_received(&self) -> u64 {
        lock(&self.state).pongs_received
    }

    /// Every token this hub was offered, in order.
    pub fn tokens_seen(&self) -> Vec<String> {
        lock(&self.state).tokens_seen.clone()
    }

    pub fn stop(&self) {
        self.close_all(4003);
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for FakeHub {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Bytes already read from the socket, then the socket.
struct Replay {
    pre: io::Cursor<Vec<u8>>,
    tcp: TcpStream,
}

impl Read for Replay {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if (self.pre.position() as usize) < self.pre.get_ref().len() {
            return self.pre.read(buf);
        }
        self.tcp.read(buf)
    }
}

impl Write for Replay {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tcp.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

#[allow(clippy::result_large_err)] // tungstenite's callback signature
fn serve(mut tcp: TcpStream, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    let _ = tcp.set_nonblocking(false);
    // -- WP LK2 -- as the ESP32 bridge (http_server.c:745): no Nagle delay on
    // replies, which matters once several commands are on the wire
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
    // Read the request head to tell the info endpoint from the upgrade
    let mut head = Vec::new();
    let mut b = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16384 {
        match tcp.read(&mut b) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&b[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if !text.contains("upgrade: websocket") {
        let s = lock(&state);
        let body = json!({
            "proto": {"major": 1, "minor": s.cfg.proto_minor},
            "hub": {"id": s.cfg.hub_id, "name": s.cfg.name, "kind": s.cfg.kind, "version": "0.0-fake"},
            "auth": if s.cfg.auth == Auth::None { "none" } else { "pin" }, "tls": false, "ws": "/dspi/v1",
            "devices": if s.board == Board::Absent { json!([]) } else { json!([{"serial": s.cfg.serial, "name": s.cfg.device_name}]) },
        })
        .to_string();
        drop(s);
        let status = if text.starts_with("get /dspi/v1/info ") { "200 OK" } else { "404 Not Found" };
        let _ = write!(
            tcp,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        return;
    }
    // Refusals come before the upgrade (http_server.c `refuse_handshake`)
    let peer = tcp.peer_addr().ok().map(|a| a.ip());
    let (slot, tx_rx) = {
        let mut s = lock(&state);
        let refuse = if let Some(code) = s.faults.refuse_handshake {
            Some(code)
        } else if s.cfg.kind == "bridge" && !text.contains("dspi-link-1") {
            Some(400)
        } else if s.cfg.max_sessions.is_some_and(|m| s.sessions.len() >= m) {
            Some(503)
        } else {
            None
        };
        if let Some(code) = refuse {
            drop(s);
            refuse_handshake(&mut tcp, code);
            return;
        }
        let sid = s.next_session;
        s.next_session = s.next_session.wrapping_add(1);
        let (tx, rx): (Sender<Out>, Receiver<Out>) = mpsc::channel();
        s.sessions.insert(
            sid,
            SessionRec {
                tx,
                role: "admin".into(),
                authed: false,
                token: None,
                link_seq: 0,
                poll_seq: 0,
                polls: HashMap::new(),
                peer,
                held: Vec::new(),
            },
        );
        (sid, rx)
    };
    let raw = tcp.try_clone();
    let stream = Replay { pre: io::Cursor::new(head), tcp };
    let ws = tungstenite::accept_hdr(stream, |req: &Request, mut resp: Response| {
        let asked = req.headers().get("Sec-WebSocket-Protocol").and_then(|v| v.to_str().ok()).unwrap_or("");
        if asked.split(',').any(|p| p.trim() == "dspi-link-1") {
            resp.headers_mut().insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        }
        Ok(resp)
    });
    if let (Ok(mut ws), Ok(raw)) = (ws, raw) {
        let _ = raw.set_read_timeout(Some(Duration::from_millis(5)));
        session(&mut ws, &state, &stop, slot, tx_rx);
    }
    let mut s = lock(&state);
    s.sessions.remove(&slot);
    if s.lock.is_some_and(|l| l.0 == slot) {
        s.lock = None;
    }
}

fn refuse_handshake(tcp: &mut TcpStream, code: u16) {
    let (line, body) = match code {
        400 => ("400 Bad Request", "Link needs the dspi-link-1 sub-protocol."),
        403 => ("403 Forbidden", "This bridge accepts Link sessions only from its own pages."),
        503 => ("503 Service Unavailable", "This bridge has no room for another session."),
        _ => ("500 Internal Server Error", "Refused."),
    };
    let _ = write!(
        tcp,
        "HTTP/1.1 {line}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn send_close(ws: &mut WebSocket<Replay>, code: u16) {
    let _ = ws.close(Some(CloseFrame { code: CloseCode::from(code), reason: "".into() }));
    let _ = ws.flush();
}

/// Write one message; `false` when the session is over.
fn write_out(ws: &mut WebSocket<Replay>, o: Out) -> bool {
    match o {
        Out::Text(t) => ws.send(Message::text(t)).is_ok(),
        Out::Binary(b) => ws.send(Message::binary(b)).is_ok(),
        Out::Close(code) => {
            send_close(ws, code);
            false
        }
        Out::Abort => {
            let _ = ws.get_ref().tcp.shutdown(Shutdown::Both);
            false
        }
    }
}

/// A reply, through the injected faults; `false` when the session is over.
fn reply_out(ws: &mut WebSocket<Replay>, state: &Arc<Mutex<State>>, sid: u16, o: Out) -> bool {
    let duplicate = {
        let mut s = lock(state);
        if s.faults.drop_replies > 0 {
            s.faults.drop_replies -= 1;
            return true;
        }
        if s.faults.hold_replies {
            if let Some(r) = s.sessions.get_mut(&sid) {
                r.held.push(o);
            }
            return true;
        }
        s.faults.duplicate
    };
    if duplicate && !write_out(ws, o.clone()) {
        return false;
    }
    write_out(ws, o)
}

fn session(ws: &mut WebSocket<Replay>, state: &Arc<Mutex<State>>, stop: &AtomicBool, sid: u16, rx: Receiver<Out>) {
    let clock = Arc::clone(&lock(state).cfg.clock);
    let started = clock.now();
    let mut last_ping = started;
    let mut last_activity = started;
    'outer: while !stop.load(Ordering::SeqCst) {
        if lock(state).faults.stalled {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        // Outgoing queue
        while let Ok(o) = rx.try_recv() {
            let duplicate = lock(state).faults.duplicate && matches!(o, Out::Text(_) | Out::Binary(_));
            if duplicate && !write_out(ws, o.clone()) {
                break 'outer;
            }
            if !write_out(ws, o) {
                break 'outer;
            }
        }
        // Polls due
        let due: Vec<(u8, Vec<u8>)> = {
            let mut s = lock(state);
            let now = s.now();
            let mut due = Vec::new();
            if let Some(r) = s.sessions.get_mut(&sid) {
                for (slot, p) in r.polls.iter_mut() {
                    if now >= p.next {
                        p.next = now + p.every;
                        due.push((*slot, p.spec.clone()));
                    }
                }
            }
            due.into_iter()
                .map(|(slot, spec)| {
                    let (_, data) = s.device(
                        sid,
                        Dir::Get,
                        spec[0],
                        u16::from_le_bytes([spec[1], spec[2]]),
                        u16::from_le_bytes([spec[3], spec[4]]),
                        &[],
                    );
                    (slot, data)
                })
                .collect()
        };
        for (slot, payload) in due {
            let seq = {
                let mut s = lock(state);
                let Some(r) = s.sessions.get_mut(&sid) else { break 'outer };
                r.poll_seq = r.poll_seq.wrapping_add(1);
                r.poll_seq
            };
            let f = Frame::Poll(PollFrame { seq, handle: 0, slot, payload });
            if ws.send(Message::binary(f.encode())).is_err() {
                break 'outer;
            }
        }
        {
            let now = clock.now();
            let mut s = lock(state);
            let authed = s.sessions.get(&sid).is_some_and(|r| r.authed);
            if !authed && now.saturating_duration_since(started) >= s.cfg.auth_deadline {
                drop(s);
                send_close(ws, 4001);
                break;
            }
            // Keepalive runs on open sessions only (hub.c `hub_tick`)
            if authed && now.saturating_duration_since(last_activity) >= s.cfg.peer_timeout {
                drop(s);
                send_close(ws, 1001);
                break;
            }
            if authed && now.saturating_duration_since(last_ping) >= s.cfg.ping_interval {
                last_ping = now;
                s.pings_sent += 1;
                drop(s);
                let _ = ws.send(Message::Ping(Vec::new().into()));
            }
        }
        let read = ws.read();
        // The network went dark during that read: the frame never got here
        if read.is_ok() && lock(state).faults.stalled {
            continue;
        }
        if read.is_ok() {
            last_activity = clock.now();
        }
        match read {
            Ok(Message::Text(t)) => {
                for r in handle_json(state, sid, t.as_str()) {
                    if !reply_out(ws, state, sid, Out::Text(r)) {
                        break 'outer;
                    }
                }
            }
            Ok(Message::Pong(_)) => lock(state).pongs_received += 1,
            Ok(Message::Binary(b)) => {
                // RATE_LIMITED is the hub's own answer: it waits for no device
                let mut at_once = false;
                let reply = {
                    let mut s = lock(state);
                    let authed = s.sessions.get(&sid).is_some_and(|r| r.authed);
                    match Frame::decode(&b) {
                        Ok(Frame::CmdRequest(f)) if authed => {
                            let r = s.command(sid, f);
                            at_once = r.status == status::RATE_LIMITED;
                            Some(Frame::CmdResponse(r).encode())
                        }
                        Ok(Frame::Unknown { kind: k, tag }) if authed && k < 0x80 => Some(
                            Frame::CmdResponse(CmdResponse { tag, status: status::UNSUPPORTED, payload: vec![] })
                                .encode(),
                        ),
                        Err(_) if authed && b.len() >= 4 && b[0] == kind::CMD_REQUEST => Some(
                            Frame::CmdResponse(CmdResponse {
                                tag: u16::from_le_bytes([b[2], b[3]]),
                                status: status::BAD_FRAME,
                                payload: vec![],
                            })
                            .encode(),
                        ),
                        _ => None,
                    }
                };
                if let Some(r) = reply {
                    let sent =
                        if at_once { write_out(ws, Out::Binary(r)) } else { reply_out(ws, state, sid, Out::Binary(r)) };
                    if !sent {
                        break;
                    }
                }
            }
            Ok(Message::Close(_)) => {
                let _ = ws.flush();
                break;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    if stop.load(Ordering::SeqCst) {
        send_close(ws, 4003);
    }
}

fn ok(id: &Value, extra: Value) -> String {
    let mut v = json!({"t": "ok", "id": id});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v.to_string()
}

fn err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

/// An `err` with no `msg`, as the bridge answers a failed credential (`hub.c`).
fn err_bare(id: &Value, code: &str) -> String {
    json!({"t": "err", "id": id, "code": code}).to_string()
}

const RATE_LIMITED_MSG: &str = "too many failures; try again in a minute";

fn handle_json(state: &Arc<Mutex<State>>, sid: u16, text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return vec![] };
    let t = v["t"].as_str().unwrap_or("");
    let id = v["id"].clone();
    let mut s = lock(state);
    let authed = s.sessions.get(&sid).is_some_and(|r| r.authed);
    match t {
        "hello" => {
            let mut h = json!({
                "t": "hello", "proto": {"major": 1, "minor": s.cfg.proto_minor},
                "hub": {"id": s.cfg.hub_id, "name": s.cfg.name, "kind": s.cfg.kind, "version": "0.0-fake"},
                "auth": if s.cfg.auth == Auth::None { "none" } else { "pin" },
                "caps": s.cfg.caps,
                "limits": {"max_frame": if s.cfg.bridge_limits { 16384 } else { 65536 }, "max_payload": 8192,
                           "max_inflight": s.cfg.max_inflight, "poll_max_hz": POLL_MAX_HZ, "poll_budget_bps": s.cfg.poll_budget_bps},
            });
            if let Auth::Fixed(_) = s.cfg.auth {
                h["pin_source"] = json!("fixed");
            }
            if s.cfg.auth == Auth::None {
                if let Some(r) = s.sessions.get_mut(&sid) {
                    r.authed = true;
                    r.role = "admin".into();
                }
            }
            vec![h.to_string()]
        }
        "auth.pair" => {
            let pin = v["pin"].as_str().unwrap_or("");
            let peer = s.peer_of(sid);
            let open = s.cfg.auth == Auth::None;
            if !open && s.rate_limited(peer) {
                return vec![err(&id, "rate_limited", RATE_LIMITED_MSG)];
            }
            let window = matches!(s.cfg.auth, Auth::Window(_));
            let (good, role) = match &s.cfg.auth {
                Auth::None => (true, "admin".to_owned()),
                // The hub decides the role (the requested one is a wish, spec 6.2)
                Auth::Window(p) => (pin == p && s.window_open, s.cfg.pin_role.clone()),
                Auth::Fixed(p) => (pin == p, s.cfg.pin_role.clone()),
            };
            if !good {
                s.auth_failed(peer);
                if window {
                    s.window_failures += 1;
                    if s.window_failures >= 5 {
                        s.window_open = false;
                    }
                }
                return vec![err_bare(&id, "unauthenticated")];
            }
            if window {
                s.window_open = false;
            }
            let token = if s.cfg.auth == Auth::None { None } else { Some(format!("tok-{sid}-{}", s.tokens.len())) };
            if let Some(tok) = &token {
                s.tokens.insert(tok.clone(), role.clone());
            }
            let denied = s.denied(&role);
            if let Some(r) = s.sessions.get_mut(&sid) {
                r.authed = true;
                r.role = role.clone();
                r.token = token.clone();
            }
            let mut extra = json!({"session": sid, "role": role, "policy": {"denied": denied}});
            if let Some(tok) = token {
                extra["token"] = json!(tok);
            }
            vec![ok(&id, extra)]
        }
        "auth.token" => {
            let tok = v["token"].as_str().unwrap_or("").to_owned();
            s.tokens_seen.push(tok.clone());
            if let Some(code) = s.faults.token_error.clone() {
                return vec![err(&id, &code, "injected")];
            }
            if s.cfg.auth == Auth::None {
                if let Some(r) = s.sessions.get_mut(&sid) {
                    r.authed = true;
                }
                return vec![ok(&id, json!({"session": sid, "role": "admin", "policy": {"denied": []}}))];
            }
            let peer = s.peer_of(sid);
            if s.rate_limited(peer) {
                return vec![err(&id, "rate_limited", RATE_LIMITED_MSG)];
            }
            match s.tokens.get(&tok).cloned() {
                Some(role) => {
                    let denied = s.denied(&role);
                    if let Some(r) = s.sessions.get_mut(&sid) {
                        r.authed = true;
                        r.role = role.clone();
                        r.token = Some(tok);
                    }
                    vec![ok(&id, json!({"session": sid, "role": role, "policy": {"denied": denied}}))]
                }
                None => {
                    s.auth_failed(peer);
                    vec![err_bare(&id, "unauthenticated")]
                }
            }
        }
        _ if !authed => vec![err(&id, "unauthenticated", "")],
        "device.list" => {
            if s.faults.device_list_times > 0 {
                s.faults.device_list_times -= 1;
                match s.faults.device_list.clone() {
                    Some(DeviceListFault::Error(code)) => return vec![err(&id, &code, "injected")],
                    Some(DeviceListFault::Silent) => return vec![],
                    None => {}
                }
            }
            let devices = if s.board == Board::Absent { vec![] } else { vec![s.device_json()] };
            vec![ok(&id, json!({"devices": devices}))]
        }
        "device.snapshot" => {
            if v["handle"].as_u64() != Some(0) || s.board != Board::Online {
                return vec![err(&id, "no_device", "")];
            }
            let b64 = data_encoding::BASE64.encode(&s.image);
            let st = data_encoding::BASE64.encode(STATUS_WVALUE9);
            vec![ok(
                &id,
                json!({"handle": 0, "wire_version": s.image[0], "age_ms": s.cfg.snapshot_age_ms, "bulk_b64": b64, "status_b64": st}),
            )]
        }
        "poll.subscribe" => {
            if v["handle"].as_u64() != Some(0) || s.board != Board::Online {
                return vec![err(&id, "no_device", "")];
            }
            let mut requests = Vec::new();
            let role = s.sessions.get(&sid).map(|r| r.role.clone()).unwrap_or_default();
            let polls = v["polls"].as_array().cloned().unwrap_or_default();
            for p in polls {
                let (slot, rq) = (p["slot"].as_u64().unwrap_or(0) as u8, p["req"].as_u64().unwrap_or(0) as u8);
                if s.class(rq, 1) != "read" || !allowed(&role, "read") {
                    return vec![err(&id, "denied", "poll commands must be read")];
                }
                let hz = u32::try_from(p["hz"].as_u64().unwrap_or(1)).unwrap_or(u32::MAX).max(1);
                let val = p["val"].as_u64().unwrap_or(0) as u16;
                let len = p["len"].as_u64().unwrap_or(0) as u16;
                let mut spec = vec![rq];
                spec.extend_from_slice(&val.to_le_bytes());
                spec.extend_from_slice(&len.to_le_bytes());
                requests.push((slot, spec, hz));
            }
            let Some(rates) = s.grant_polls(sid, &requests) else {
                return vec![err(&id, "busy", "the poll budget cannot fit these polls even at 1 Hz")];
            };
            let now = s.now();
            let mut granted = Vec::new();
            for ((slot, spec, _), hz) in requests.into_iter().zip(rates) {
                if let Some(r) = s.sessions.get_mut(&sid) {
                    let every = Duration::from_millis(1000 / u64::from(hz));
                    r.polls.insert(slot, PollRec { spec, hz, every, next: now });
                }
                granted.push(json!({"slot": slot, "hz": hz}));
            }
            vec![ok(&id, json!({"granted": granted}))]
        }
        "poll.unsubscribe" => {
            let slots: Vec<u8> =
                v["slots"].as_array().into_iter().flatten().filter_map(|x| x.as_u64()).map(|x| x as u8).collect();
            if let Some(r) = s.sessions.get_mut(&sid) {
                r.polls.retain(|k, _| !slots.contains(k));
            }
            vec![ok(&id, json!({}))]
        }
        "lock.acquire" => {
            let role = s.sessions.get(&sid).map(|r| r.role.clone()).unwrap_or_default();
            if role == "viewer" {
                return vec![err(&id, "denied", "viewer")];
            }
            let ms = v["timeout_ms"].as_i64().unwrap_or(10_000);
            if ms < 0 {
                return vec![err(&id, "bad_request", "negative timeout")];
            }
            let ms = ms.min(60_000) as u64;
            if let Some((holder, until)) = s.lock {
                if holder != sid && Instant::now() < until {
                    return vec![err(&id, "locked", "Another console is applying changes")];
                }
            }
            s.lock = Some((sid, Instant::now() + Duration::from_millis(ms)));
            let changed = json!({"t": "device.changed", "device": s.device_json()}).to_string();
            s.broadcast(changed);
            vec![ok(&id, json!({"timeout_ms": ms}))]
        }
        "lock.release" => {
            if s.lock.is_some_and(|l| l.0 == sid) {
                s.lock = None;
                let changed = json!({"t": "device.changed", "device": s.device_json()}).to_string();
                s.broadcast(changed);
            }
            vec![ok(&id, json!({}))]
        }
        _ => vec![err(&id, "unknown_type", t)],
    }
}
