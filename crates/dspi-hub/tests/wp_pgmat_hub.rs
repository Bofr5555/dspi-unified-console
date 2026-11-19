//! WP PGMAT: the compat matrix, our hub's side (PLAN-PG section 4): section 1.11's
//! "OH (our hub)" column for our clients, and its second table, our hub with
//! upstream-style clients.
//!
//! The real hub on its real server (loopback), with PGFAKE1/2's fakes of every
//! generation attached (the rows of `crates/dspi-core/tests/wp_pgmat_matrix.rs`,
//! whose `common.rs` this binary shares: the axes, section 1.11's cells as outcomes,
//! the checks on a connected core and the rendered `matrix.md`):
//! - **our client**: `DspiCore` over DSPi Link, a control-role session (a PIN
//!   pairing), the extension channel also tried by an admin (open access); for the
//!   paged DSPi-Zero also the app's in-process path and a principal's child, the
//!   three cores that learn its board through `DeviceDescriptor::hub_board`;
//! - **an L12-style client**: hello minor 2 (the 1.2 fixture `hello_client.json`
//!   with the minor our 1.2 hub page and older apps send) and 1.1.6-shaped CMD
//!   frames encoded by `dspi-link`;
//! - **an L13 client**: DSPi-Link 1.3's own fixtures replayed
//!   (`tests/fixtures/link/upstream-1.3/`, DSPi-Link `4aec5ab`, MIT): its hello, its
//!   `device.list`, its CMD frames (`ROUTE.MASTER_VOLUME` SET, `SYS.PLATFORM` GET; the
//!   handle byte set to the attached device's), its `poll.subscribe`.
//!
//! On a Zero both upstream-style clients also send a bootloader entry, which the
//! router refuses (`DENIED`); every Zero cell is "blocked", as the plan wants.
//! The fakes fail a test by themselves on a violation of section 1.1; each cell
//! also asserts none happened.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::arithmetic_side_effects
)]

#[path = "../../dspi-core/tests/wp_pgmat_matrix/common.rs"]
mod common;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use common::*;
use dspi_core::{ConfirmOp, DspiCore, DspiError};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_link::frames::{CmdRequest, CmdResponse, Frame};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, Dir, LinkClient, status};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::packets::Board;
use dspi_proto::paged::legacy::VC_ERR_UNKNOWN;
use dspi_proto::paged::{CmdId, EXT_TUNNEL, cmds};
use dspi_transport::{FakeRemote, Generation, paged};
use serde_json::{Value, json};
use tungstenite::protocol::Role as WsRole;
use tungstenite::{Message, WebSocket};

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

// ── The hub ─────────────────────────────────────────────────────────────────

struct Bench {
    hub: Hub,
    port: u16,
}

impl Bench {
    /// Our hub on loopback with `access`, one fake of `fw` attached.
    fn start(fw: Fw, access: AccessMode, allow: bool) -> (Self, FakeRemote, u8) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
        let dir = std::env::temp_dir().join("wp-pgmat").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let hub = Hub::start(HubConfig {
            network: Some(NetworkConfig {
                port: 0,
                access,
                bind: LOOPBACK,
                ..NetworkConfig::new(dir)
            }),
            ..HubConfig::in_process("PGMAT hub")
        })
        .unwrap();
        let dev = fw.device(allow);
        let remote = dev.remote();
        let handle = hub
            .attach(Box::new(dev), 0)
            .unwrap_or_else(|e| panic!("{fw:?}: attach: {e}"));
        let port = hub.network().unwrap().port();
        (Self { hub, port }, remote, handle)
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/dspi/v1", self.port)
    }

    fn pin(&self) -> String {
        self.hub
            .network()
            .unwrap()
            .auth()
            .begin_pairing(Instant::now())
            .unwrap()
            .pin
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

// ── Our client on our hub ───────────────────────────────────────────────────

/// Our core on the bench as a control session (PIN access) or admin (open
/// access); the device selected. The selection's result.
fn our_core(b: &Bench, control: bool) -> (Arc<DspiCore>, Result<(), DspiError>) {
    let core = DspiCore::new();
    core.link_connect(
        b.url(),
        "wp-pgmat".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    if control {
        core.link_pair(b.pin()).unwrap();
        assert_eq!(core.link_hub_role(), "control");
    }
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    let selected = core.link_select_device(d.handle, d.serial);
    (core, selected)
}

/// The device at the transport through our hub: our Link client, `LinkTransport`,
/// `paged::open`. Its generation, and whether `EXT_LIST` answers.
fn at_the_transport(fw: Fw, control: bool) -> (bool, bool) {
    let access = if control {
        AccessMode::Pin
    } else {
        AccessMode::Open
    };
    let (b, remote, _) = Bench::start(fw, access, false);
    let options = ConnectOptions {
        client_name: "wp-pgmat".into(),
        app: "dspi-hub tests".into(),
        app_version: "test".into(),
        token: None,
    };
    let (client, state) =
        LinkClient::connect(&TcpConnector::default(), &b.url(), &options, None).unwrap();
    if !matches!(state, AuthState::Ready) {
        client.pair(&b.pin(), "control").unwrap();
    }
    wait_until("the device list", || !client.devices().is_empty());
    let d = client.devices().remove(0);
    let link = dspi_transport::link::LinkTransport::new(
        client.clone(),
        u8::try_from(d.handle).unwrap(),
        &d.serial,
    );
    let mut t = paged::open(Box::new(link)).unwrap();
    let paged = matches!(t.generation(), Some(Generation::Paged(_)));
    let ext = dspi_session::ext::list(t.as_mut()).is_ok_and(|ids| !ids.is_empty());
    drop(t);
    client.close();
    no_violation(fw, Route::Oh, &remote, 0);
    (paged, ext)
}

/// A paged Zero's board through the hub's two in-process paths (the app's local
/// transport, a principal's child): each core on top knows it and refuses the
/// bootloader entry itself.
fn zero_in_process(b: &Bench, remote: &FakeRemote, notes: &mut Notes) {
    let serial = b.hub.devices()[0].identity.serial.clone();
    let h = b.hub.handle();
    let check = |core: &DspiCore, path: &str| {
        assert_eq!(core.board(), Ok(Board::PiZero2W), "{path}");
        assert!(!core.firmware_target().has_bootsel(), "{path}");
        let entry =
            core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)));
        assert_eq!(entry, Err(DspiError::Unsupported), "{path}");
        assert_eq!(remote.rebooted_to(), None, "{path}");
    };
    let local = h.open_local(&serial).unwrap();
    let core = DspiCore::new();
    core.open_transport(Box::new(local), serial.clone(), 0)
        .unwrap();
    check(&core, "local");
    core.disconnect();
    let parent = h.open_session(Role::Admin, None).unwrap();
    let child = h.open_child(&parent).unwrap();
    let principal = h.open_as(&serial, &child, true).unwrap();
    let core = DspiCore::new();
    core.open_transport(Box::new(principal), serial, 0).unwrap();
    check(&core, "principal");
    core.disconnect();
    notes.extra.push(
        "the app's local path and a principal's child: board 1 \"DSPi Zero\", no BOOTSEL, \
         the gated entry refused"
            .into(),
    );
}

fn our_client(fw: Fw) -> Cell {
    let route = Route::Oh;
    let expected = expect(fw, route);
    let mut notes = Notes::default();
    let (b, remote, _) = Bench::start(fw, AccessMode::Pin, false);
    let (core, selected) = our_core(&b, true);
    let observed = if let Err(e) = selected {
        // The core does not connect (the sender's vendor subset).
        assert_eq!(fw, Fw::Sender, "{fw:?}: select: {e}");
        notes.connect = format!("the core's connect sequence: {e}");
        core.link_close();
        no_violation(fw, route, &remote, 0);
        let (paged, control_ext) = at_the_transport(fw, true);
        let (_, admin_ext) = at_the_transport(fw, false);
        notes.ext = format!(
            "at the transport, control: EXT_LIST {}, admin: {}",
            if control_ext { "answers" } else { "refused" },
            if admin_ext { "answers" } else { "refused" }
        );
        Outcome::NoCore {
            paged,
            ext: ext_of(control_ext, admin_ext),
        }
    } else {
        core.fetch_all().unwrap();
        let (paged, control_ext, boot, bulk, generator) =
            exercise(fw, route, &core, &remote, &mut notes);
        core.link_close();
        no_violation(fw, route, &remote, 0);
        if fw == Fw::ZeroPaged {
            zero_in_process(&b, &remote, &mut notes);
        }
        // The extension channel as admin (open access).
        let (admin_bench, admin_remote, _) = Bench::start(fw, AccessMode::Open, false);
        let (admin, selected) = our_core(&admin_bench, false);
        selected.unwrap_or_else(|e| panic!("{fw:?}: admin select: {e}"));
        let admin_ext = !admin.ext_status().unwrap().supported.is_empty();
        admin.link_close();
        no_violation(fw, route, &admin_remote, 0);
        notes.ext = format!(
            "control: {}, admin: {}",
            if control_ext { "listed" } else { "empty" },
            if admin_ext { "listed" } else { "empty" }
        );
        Outcome::Full {
            paged,
            ext: ext_of(control_ext, admin_ext),
            boot,
            bulk,
            generator,
        }
    };
    Cell {
        fw,
        route,
        expected,
        observed,
        notes,
    }
}

#[test]
fn wp_pgmat_our_client_on_our_hub() {
    let cells: Vec<Cell> = ROWS.iter().map(|&fw| our_client(fw)).collect();
    check("hub-oh", &cells);
}

// ── Upstream-style clients on our hub ───────────────────────────────────────

type Ws = WebSocket<TcpStream>;

fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/link")
        .join(rel)
}

fn json_fixture(rel: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture_path(rel)).unwrap()).unwrap()
}

/// A raw WebSocket to the bench, as any client opens one.
fn ws(port: u16) -> Ws {
    let mut tcp = TcpStream::connect((LOOPBACK, port)).unwrap();
    tcp.set_read_timeout(Some(WAIT)).unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.write_all(
        format!(
            "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
             Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        match tcp.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    WebSocket::from_raw_socket(tcp, WsRole::Client, None)
}

fn next_text(ws: &mut Ws, t: &[&str]) -> Value {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no {t:?} in time");
        match ws.read().unwrap() {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if t.iter().any(|t| v["t"] == *t) {
                    return v;
                }
            }
            Message::Close(frame) => panic!("closed while waiting for {t:?}: {frame:?}"),
            _ => {}
        }
    }
}

fn request(ws: &mut Ws, value: Value) -> Value {
    ws.send(Message::text(value.to_string())).unwrap();
    next_text(ws, &["ok", "err"])
}

fn command(ws: &mut Ws, req: CmdRequest) -> CmdResponse {
    let tag = req.tag;
    ws.send(Message::binary(Frame::CmdRequest(req).encode()))
        .unwrap();
    loop {
        match ws.read().unwrap() {
            Message::Binary(bytes) => {
                if let Ok(Frame::CmdResponse(r)) = Frame::decode(&bytes)
                    && r.tag == tag
                {
                    return r;
                }
            }
            Message::Close(frame) => panic!("closed: {frame:?}"),
            _ => {}
        }
    }
}

/// A raw client on the bench (open access: admin) that said `hello` and listed the
/// devices; the hub's hello and the device object.
fn raw_client(b: &Bench, hello: Value) -> (Ws, Value, Value) {
    let mut ws = ws(b.port);
    ws.send(Message::text(hello.to_string())).unwrap();
    let hub_hello = next_text(&mut ws, &["hello"]);
    let list = request(
        &mut ws,
        json_fixture("upstream-1.3/messages/device_list.json"),
    );
    assert_eq!(list["t"], "ok", "{list}");
    let device = list["devices"][0].clone();
    (ws, hub_hello, device)
}

/// A 1.1.6-shaped request (page 0, interface 2).
fn legacy_req(
    tag: u16,
    handle: u8,
    dir: Dir,
    opcode: u8,
    value: u16,
    payload: Vec<u8>,
    len: u16,
) -> CmdRequest {
    CmdRequest {
        tag,
        handle,
        dir,
        b_request: opcode,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: if dir == Dir::Get {
            len
        } else {
            u16::try_from(payload.len()).unwrap()
        },
        payload,
    }
}

/// A request by command id, as a 1.3 client sends it.
fn paged_req(
    tag: u16,
    handle: u8,
    dir: Dir,
    id: CmdId,
    value: u16,
    payload: Vec<u8>,
    len: u16,
) -> CmdRequest {
    CmdRequest {
        w_index: id.w_index(),
        b_request: id.cmd(),
        ..legacy_req(tag, handle, dir, 0, value, payload, len)
    }
}

/// A fixture's CMD frame, its handle set to the attached device's.
fn frame_fixture(rel: &str, handle: u8) -> CmdRequest {
    let bytes = std::fs::read(fixture_path(rel)).unwrap();
    let Frame::CmdRequest(req) = Frame::decode(&bytes).unwrap() else {
        panic!("{rel} is not a CMD request")
    };
    CmdRequest { handle, ..req }
}

fn db(v: f32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

/// An L12-style client: section 1.11's "L12 client" row.
fn l12_client(fw: Fw) -> Cell {
    let route = Route::OhL12Client;
    let expected = expect(fw, route);
    let mut notes = Notes::default();
    let (b, remote, h) = Bench::start(fw, AccessMode::Open, false);
    let mut hello = json_fixture("messages/hello_client.json");
    hello["proto"]["minor"] = json!(dspi_link::messages::PROTO_MINOR);
    assert_eq!(dspi_link::messages::PROTO_MINOR, 2);
    let (mut ws, _, device) = raw_client(&b, hello);
    let translated = device.get("protocol").is_some_and(|p| !p.is_null());
    notes.connect = format!(
        "listed with protocol: {}",
        device.get("protocol").unwrap_or(&Value::Null)
    );

    let platform = command(
        &mut ws,
        legacy_req(1, h, Dir::Get, op::REQ_GET_PLATFORM, 0, Vec::new(), 7),
    );
    assert_eq!(platform.status, status::OK, "{fw:?}: GET_PLATFORM");
    assert!(platform.payload.len() >= 4, "{fw:?}");
    if fw != Fw::Sender {
        let set = command(
            &mut ws,
            legacy_req(2, h, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, db(-21.0), 0),
        );
        assert_eq!(set.status, status::OK, "{fw:?}: SET_MASTER_VOLUME");
        let get = command(
            &mut ws,
            legacy_req(3, h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
        );
        assert_eq!((get.status, get.payload), (status::OK, db(-21.0)), "{fw:?}");
        notes.write = "SET_MASTER_VOLUME -21 dB, read back".into();
    } else {
        notes.write = "the sender: GET_PLATFORM only".into();
    }
    let ext = command(
        &mut ws,
        legacy_req(
            4,
            h,
            Dir::Get,
            op::REQ_EXTENSION,
            gext::EXT_LIST,
            Vec::new(),
            64,
        ),
    );
    let has_ext = ext.status == status::OK && !ext.payload.is_empty();
    notes.ext = format!(
        "0x00 EXT_LIST: status 0x{:02X}, {} bytes",
        ext.status,
        ext.payload.len()
    );
    let entry = fw.is_zero().then(|| {
        let r = command(
            &mut ws,
            legacy_req(5, h, Dir::Get, op::REQ_ENTER_BOOTLOADER, 0, Vec::new(), 1),
        );
        assert_eq!(r.status, status::DENIED, "{fw:?}: 0xF0");
        assert_eq!(remote.rebooted_to(), None);
        notes.boot = "0xF0 answered DENIED, nothing sent".into();
        Boot::Blocked
    });
    no_violation(fw, route, &remote, 0);
    Cell {
        fw,
        route,
        expected,
        observed: Outcome::Up {
            kind: if translated {
                UpKind::Translated
            } else {
                UpKind::AsToday
            },
            ext: has_ext,
            entry,
        },
        notes,
    }
}

/// An L13 client replaying DSPi-Link 1.3's fixtures: section 1.11's "L13 client" row.
fn l13_client(fw: Fw) -> Cell {
    let route = Route::OhL13Client;
    let expected = expect(fw, route);
    let mut notes = Notes::default();
    let (b, remote, h) = Bench::start(fw, AccessMode::Open, false);
    let hello = json_fixture("upstream-1.3/messages/hello_client.json");
    assert_eq!(hello["proto"]["minor"], 3, "the 1.3 fixture");
    let (mut ws, hub_hello, device) = raw_client(&b, hello);
    assert_eq!(hub_hello["proto"]["minor"], 3, "our hub says 1.3 (R4)");
    let protocol = device.get("protocol").filter(|p| !p.is_null()).cloned();
    notes.connect = format!(
        "listed with protocol: {}",
        protocol.clone().unwrap_or(Value::Null)
    );
    let Some(protocol) = protocol else {
        // A 1.3 client sends a device without `protocol` nothing (spec 1.3 7.4).
        let sent = remote.paged_requests();
        no_violation(fw, route, &remote, 0);
        assert_eq!(sent, 1, "{fw:?}: only our hub's attach probe");
        return Cell {
            fw,
            route,
            expected,
            observed: Outcome::Up {
                kind: UpKind::Unlisted,
                ext: false,
                entry: None,
            },
            notes,
        };
    };
    assert_eq!(protocol["major"], 1, "{fw:?}");
    // The device object has the fixture's shape (every key of `device_list_ok.json`).
    let want = json_fixture("upstream-1.3/messages/device_list_ok.json");
    for key in want["devices"][0].as_object().unwrap().keys() {
        if ["inputs", "wire_version"].contains(&key.as_str()) {
            continue; // left out when not read, as spec 7.4 allows (HB3)
        }
        assert!(device.get(key).is_some(), "{fw:?}: no `{key}` in {device}");
    }

    let set = command(
        &mut ws,
        frame_fixture("upstream-1.3/frames/cmd_request_set_master_volume.bin", h),
    );
    assert_eq!(
        set.status,
        status::OK,
        "{fw:?}: the fixture's ROUTE.MASTER_VOLUME SET"
    );
    let back = command(
        &mut ws,
        paged_req(20, h, Dir::Get, cmds::ROUTE_MASTER_VOLUME, 0, Vec::new(), 4),
    );
    assert_eq!(
        (back.status, back.payload),
        (status::OK, db(-20.0)),
        "{fw:?}"
    );
    notes.write = "the fixture's ROUTE.MASTER_VOLUME -20 dB, read back by page".into();
    let platform = command(
        &mut ws,
        frame_fixture("upstream-1.3/frames/cmd_request_get_platform.bin", h),
    );
    assert_eq!(
        platform.status,
        status::OK,
        "{fw:?}: the fixture's SYS.PLATFORM"
    );
    assert_eq!(platform.payload.len(), 7, "{fw:?}");
    let mut poll = json_fixture("upstream-1.3/messages/poll_subscribe.json");
    poll["handle"] = json!(h);
    let polled = request(&mut ws, poll);
    notes
        .extra
        .push(format!("the fixture's poll.subscribe: {}", polled["t"]));
    assert_eq!(polled["t"], "ok", "{fw:?}: {polled}");
    let page0 = command(
        &mut ws,
        legacy_req(21, h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
    );
    assert_eq!(
        page0.status, VC_ERR_UNKNOWN,
        "{fw:?}: a page-0 request is 0x10"
    );
    let tunnel = command(
        &mut ws,
        paged_req(22, h, Dir::Get, EXT_TUNNEL, gext::EXT_LIST, Vec::new(), 64),
    );
    let ext = tunnel.status == status::OK && !tunnel.payload.is_empty();
    notes.ext = format!(
        "0xE010 EXT_LIST: status 0x{:02X}, {} bytes",
        tunnel.status,
        tunnel.payload.len()
    );
    let entry = fw.is_zero().then(|| {
        let r = command(
            &mut ws,
            paged_req(
                23,
                h,
                Dir::Set,
                cmds::SYS_ENTER_BOOTLOADER,
                0,
                Vec::new(),
                0,
            ),
        );
        assert_eq!(r.status, status::DENIED, "{fw:?}: 0x0130");
        assert_eq!(remote.rebooted_to(), None);
        notes.boot = "0x0130 answered DENIED, nothing sent".into();
        Boot::Blocked
    });
    no_violation(fw, route, &remote, 0);
    Cell {
        fw,
        route,
        expected,
        observed: Outcome::Up {
            kind: UpKind::ByPage,
            ext,
            entry,
        },
        notes,
    }
}

#[test]
fn wp_pgmat_an_l12_client_on_our_hub() {
    let cells: Vec<Cell> = ROWS.iter().map(|&fw| l12_client(fw)).collect();
    check("hub-oh-l12-client", &cells);
}

#[test]
fn wp_pgmat_an_l13_client_replaying_the_13_fixtures_on_our_hub() {
    let cells: Vec<Cell> = ROWS.iter().map(|&fw| l13_client(fw)).collect();
    check("hub-oh-l13-client", &cells);
}
