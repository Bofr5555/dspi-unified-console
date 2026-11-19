//! WP PGMAT: the compat matrix, our clients' side (PLAN-PG section 4, the cells of
//! section 1.11's first table with the routes our core takes without our hub).
//!
//! Our core (`DspiCore`, as every client uses it) against every fake generation
//! (PGFAKE1/2: G1s, G1b with and without PAGEGUARD, the alink sender, G2s, G2b, the
//! DSPi-Zero legacy and paged, G2s at a newer page revision) over each route:
//! **USB** (the fake is the raw wire, opened through `paged::open` as every
//! construction site does), **USB MIDI** (a SysEx pipe in front of the fake, the
//! page in the frame's `wIndex`), and the fake hub's **`L12Forward`**, **`L12Drop`**
//! and **`L13`** modes. Our hub (OH) is `crates/dspi-hub/tests/wp_pgmat_hub.rs`.
//!
//! Per cell: connect, the expected notice or state read (the generation), one
//! control write read back, an extension read where expected, a bulk round trip, the
//! bootloader offered or blocked (the update check, and the gated entry where
//! blocked), a generator START where expected. Through a hub the main pass runs as a
//! **control**-role session and the extension read is also tried as **admin**, which
//! is how "ext admin-only" is told from "ext". The fakes fail a test by themselves on
//! a violation of section 1.1's hard rules; each cell also asserts the count it
//! expects (0, or the page-0 requests a paged device behind a 1.2 hub refused).
//!
//! Each cell's expectation is section 1.11's text, encoded as an `Outcome`
//! (`wp_pgmat_matrix/common.rs`, shared with the hub's binary); the
//! observation is encoded the same way and compared. The run writes its cells to
//! `$PGMAT_MATRIX_DIR` (else `$TMPDIR/pgmat`) and renders `matrix.md` there from the
//! cells of both PGMAT test binaries that are present. A cell that differs from the
//! plan fails the test unless it is listed in `common::FINDINGS` (then it must still
//! differ, so a fix shows).
#![cfg(feature = "link")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

#[path = "wp_pgmat_matrix/common.rs"]
mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use dspi_core::{ConfirmOp, DspiCore, DspiError, MidiInputHandle};
use dspi_link::fakehub::{Auth, FakeHub, HubConfig, HubDevice, SharedDevice};
use dspi_link::socket::TcpConnector;
use dspi_link::{ConnectOptions, Dir, LinkClient, status};
use dspi_proto::frame::{DEFAULT_MFR_ID, Frame, SysexDecoder, encode_sysex, status as mstatus};
use dspi_proto::generated::opcodes as op;
use dspi_proto::paged::{CmdId, LastResult, PagedDir, cmds};
use dspi_transport::link::{LinkNotice, LinkTransport};
use dspi_transport::midi::{MidiInput, MidiPipe, MidiTransport, PipeError};
use dspi_transport::{FakeDevice, FakeRemote, Generation, Transport, TransportError, paged};

const PIN: &str = "246810";

// ── USB and MIDI ────────────────────────────────────────────────────────────

/// The fake as a USB MIDI device: SysEx frames in, the vendor request on the fake
/// (page 0 as the 1.1.6 calls, a page as the paged calls), the reply framed back. A
/// refusal is status `ERROR` (0x02), as on 1.1.6, PAGEGUARD and a paged STALL.
struct MidiBridge {
    dev: Mutex<FakeDevice>,
    decoder: Mutex<SysexDecoder>,
    input: Mutex<Option<Input>>,
}

/// Where the bridge delivers the device's replies: the core's port handle, or a
/// bare `MidiTransport`'s input.
enum Input {
    Handle(Arc<MidiInputHandle>),
    Raw(MidiInput),
}

impl MidiBridge {
    fn new(dev: FakeDevice) -> (Arc<Self>, Arc<dyn MidiPipe>) {
        let bridge = Arc::new(MidiBridge {
            dev: Mutex::new(dev),
            decoder: Mutex::new(SysexDecoder::new(DEFAULT_MFR_ID)),
            input: Mutex::new(None),
        });
        let pipe: Arc<dyn MidiPipe> = bridge.clone();
        (bridge, pipe)
    }
}

impl MidiBridge {
    fn answer(&self, frame: Frame) -> Frame {
        let mut dev = self.dev.lock().unwrap();
        let refused = || mstatus::ERROR;
        match frame {
            Frame::Get {
                request,
                value,
                index,
                length,
            } => {
                let page = index.to_be_bytes()[0];
                let r = if page == 0 {
                    dev.control_in_upto(request, value, length)
                } else {
                    dev.control_in_paged(CmdId::new(page, request), value, length)
                };
                match r {
                    Ok(payload) => Frame::GetResponse {
                        status: mstatus::OK,
                        payload,
                    },
                    Err(_) => Frame::GetResponse {
                        status: refused(),
                        payload: Vec::new(),
                    },
                }
            }
            Frame::Set {
                request,
                value,
                index,
                payload,
            } => {
                let page = index.to_be_bytes()[0];
                let r = if page == 0 {
                    dev.control_out(request, value, &payload)
                } else {
                    dev.control_out_paged(CmdId::new(page, request), value, &payload)
                };
                Frame::SetResponse {
                    status: if r.is_ok() { mstatus::OK } else { refused() },
                }
            }
            _ => Frame::SetResponse {
                status: mstatus::FRAME_ERROR,
            },
        }
    }
}

impl MidiPipe for MidiBridge {
    fn send(&self, sysex: &[u8]) -> Result<(), PipeError> {
        let messages = self.decoder.lock().unwrap().feed(sysex);
        for m in messages {
            let Ok(frame) = m.frame else { continue };
            let reply = encode_sysex(DEFAULT_MFR_ID, m.tag, &self.answer(frame));
            match &*self.input.lock().unwrap() {
                Some(Input::Handle(h)) => h.receive(reply),
                Some(Input::Raw(i)) => i.receive(&reply),
                None => return Err(PipeError::Closed),
            }
        }
        Ok(())
    }
}

fn direct(fw: Fw, route: Route) -> Cell {
    let expected = expect(fw, route);
    let mut notes = Notes::default();
    if expected.outcome == Outcome::NotApplicable {
        return Cell {
            fw,
            route,
            expected,
            observed: Outcome::NotApplicable,
            notes,
        };
    }
    let dev = fw.device(false);
    let remote = dev.remote();
    let core = DspiCore::new();
    let opened = match route {
        Route::Usb => {
            let serial = dev.descriptor().serial.clone();
            let t = paged::open(Box::new(dev)).unwrap_or_else(|e| panic!("{fw:?}: {e}"));
            core.open_transport(t, serial, 0)
        }
        Route::Midi => {
            let (bridge, pipe) = MidiBridge::new(dev);
            let handle = MidiInputHandle::new(pipe, format!("MIDI-{}", fw.key()));
            *bridge.input.lock().unwrap() = Some(Input::Handle(handle.clone()));
            core.open_midi(handle)
        }
        _ => unreachable!(),
    };
    if let Err(e) = opened {
        // The core does not connect (the sender: its vendor subset); what the
        // device is at the transport, on a fresh fake.
        assert_eq!(fw, Fw::Sender, "{fw:?} {route:?}: {e}");
        notes.connect = format!("the core's connect sequence: {e}");
        no_violation(fw, route, &remote, 0);
        let (paged, ext) = at_the_transport(fw, route, &mut notes);
        return Cell {
            fw,
            route,
            expected,
            observed: Outcome::NoCore {
                paged,
                ext: if ext { Ext::Full } else { Ext::None },
            },
            notes,
        };
    }
    let (paged, ext, boot, bulk, generator) = exercise(fw, route, &core, &remote, &mut notes);
    core.disconnect();
    no_violation(fw, route, &remote, 0);
    let observed = Outcome::Full {
        paged,
        ext: if ext { Ext::Full } else { Ext::None },
        boot,
        bulk,
        generator,
    };
    Cell {
        fw,
        route,
        expected,
        observed,
        notes,
    }
}

/// The device at the transport, as `paged::open` sees it on `route`: its
/// generation, and whether `EXT_LIST` answers on it (USB and MIDI).
fn at_the_transport(fw: Fw, route: Route, notes: &mut Notes) -> (bool, bool) {
    let dev = fw.device(false);
    let remote = dev.remote();
    let raw: Box<dyn Transport> = match route {
        Route::Usb => Box::new(dev),
        Route::Midi => {
            let (bridge, pipe) = MidiBridge::new(dev);
            let (t, input) = MidiTransport::new(pipe, DEFAULT_MFR_ID, "MIDI-RAW");
            *bridge.input.lock().unwrap() = Some(Input::Raw(input));
            Box::new(t)
        }
        _ => unreachable!(),
    };
    let mut t = paged::open(raw).unwrap_or_else(|e| panic!("{fw:?} {route:?}: open: {e}"));
    let paged = matches!(t.generation(), Some(Generation::Paged(_)));
    let ext = dspi_session::ext::list(t.as_mut()).is_ok_and(|ids| !ids.is_empty());
    notes.ext = format!(
        "at the transport: {:?}, EXT_LIST {}",
        t.generation(),
        if ext { "answers" } else { "refused" }
    );
    assert_eq!(
        remote.paged_requests(),
        1,
        "{fw:?} {route:?}: the probe only"
    );
    no_violation(fw, route, &remote, 0);
    (paged, ext)
}

// ── Through the fake hub ────────────────────────────────────────────────────

/// A fake behind the fake hub as a USB hub forwards to it: page 0 as the 1.1.6
/// calls, a page as the paged calls; a paged STALL is the `SYS.LAST_RESULT` code
/// when the record names the request (a 1.3 hub, spec 1.3 section 9.1).
struct OnHub {
    dev: FakeDevice,
    explain: bool,
}

impl HubDevice for OnHub {
    fn request(
        &mut self,
        dir: Dir,
        b_request: u8,
        w_value: u16,
        w_index: u16,
        w_length: u16,
        payload: &[u8],
    ) -> (u8, Vec<u8>) {
        let page = w_index.to_be_bytes()[0];
        let id = CmdId::new(page, b_request);
        let r = match (page, dir) {
            (0, Dir::Get) => self.dev.control_in_upto(b_request, w_value, w_length),
            (0, Dir::Set) => self
                .dev
                .control_out(b_request, w_value, payload)
                .map(|()| Vec::new()),
            (_, Dir::Get) => self.dev.control_in_paged(id, w_value, w_length),
            (_, Dir::Set) => self
                .dev
                .control_out_paged(id, w_value, payload)
                .map(|()| Vec::new()),
        };
        match r {
            Ok(d) => (status::OK, d),
            Err(TransportError::Stalled { .. }) if self.explain && page != 0 => {
                let pdir = if dir == Dir::Get {
                    PagedDir::Get
                } else {
                    PagedDir::Set
                };
                match self
                    .dev
                    .control_in_paged(cmds::SYS_LAST_RESULT, 0, 8)
                    .ok()
                    .and_then(|b| LastResult::decode(&b))
                {
                    Some(l) if l.matches(id, pdir, w_value) => (l.result, Vec::new()),
                    _ => (status::ERROR, Vec::new()),
                }
            }
            Err(TransportError::Disconnected) => (status::NO_DEVICE, Vec::new()),
            Err(_) => (status::ERROR, Vec::new()),
        }
    }
}

/// The fake hub of `route` in front of a fresh fake of `fw`, granting `role` to a
/// PIN. `allow`: the fake lets a violation through (counted).
fn fake_hub(fw: Fw, route: Route, role: &str, allow: bool) -> (FakeHub, FakeRemote) {
    let dev = fw.device(allow);
    let remote = dev.remote();
    let device = SharedDevice::new(OnHub {
        dev,
        explain: route == Route::L13 && fw.is_paged(),
    });
    let cfg = match route {
        Route::L12Forward => HubConfig::l12_forward(Some(device)),
        Route::L12Drop => HubConfig::l12_drop(Some(device)),
        Route::L13 => HubConfig::l13(Some(device)),
        _ => unreachable!(),
    };
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        pin_role: role.into(),
        ..cfg
    })
    .unwrap();
    (hub, remote)
}

/// Our core paired with the hub; the device selected. The selection's result.
fn link_core(hub: &FakeHub) -> (Arc<DspiCore>, Result<(), DspiError>) {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-pgmat".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core.link_pair(PIN.into()).unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    let selected = core.link_select_device(d.handle, d.serial);
    (core, selected)
}

/// Which notice a refused selection shows (in its error or the link status).
fn notice_of(core: &DspiCore, selected: &Result<(), DspiError>) -> Option<LinkNotice> {
    let mut text = core.link_status().close_reason;
    if let Err(e) = selected {
        text.push_str(&format!(" {e} {e:?}"));
    }
    [
        LinkNotice::Needs13Hub,
        LinkNotice::HubServesPagedOnly,
        LinkNotice::GenerationChanged,
    ]
    .into_iter()
    .find(|n| text.contains(n.text()) || text.contains(n.code()))
}

/// The device at the transport through the fake hub of `route`, as a `role`
/// session: our Link client, `LinkTransport`, `paged::open`. Its generation, and
/// whether `EXT_LIST` answers.
fn over_link(fw: Fw, route: Route, role: &str) -> (bool, bool) {
    let (hub, remote) = fake_hub(fw, route, role, false);
    let options = ConnectOptions {
        client_name: "wp-pgmat".into(),
        app: "dspi-core tests".into(),
        app_version: "test".into(),
        token: None,
    };
    let (client, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &options, None)
        .unwrap_or_else(|e| panic!("{fw:?} {route:?}: {e}"));
    client.pair(PIN, role).unwrap();
    wait_until("the device list", || !client.devices().is_empty());
    let d = client.devices().remove(0);
    let link = LinkTransport::new(client.clone(), u8::try_from(d.handle).unwrap(), &d.serial);
    let mut t = paged::open(Box::new(link)).unwrap_or_else(|e| panic!("{fw:?} {route:?}: {e}"));
    let paged = matches!(t.generation(), Some(Generation::Paged(_)));
    let ext = dspi_session::ext::list(t.as_mut()).is_ok_and(|ids| !ids.is_empty());
    drop(t);
    client.close();
    let paged_sent: Vec<_> = hub
        .requests()
        .into_iter()
        .filter(|r| r.sent_page() != 0)
        .collect();
    assert!(paged_sent.is_empty(), "{fw:?} {route:?}: {paged_sent:?}");
    no_violation(fw, route, &remote, 0);
    (paged, ext)
}

fn through_hub(fw: Fw, route: Route) -> Cell {
    let expected = expect(fw, route);
    let mut notes = Notes::default();
    // A paged device behind a 1.2 hub refuses the one page-0 read our client sends
    // (a violation the fake is told to let through, and counted).
    let refuses_platform = matches!(
        expected.outcome,
        Outcome::Notice {
            sent: Sent::PlatformOnly,
            ..
        }
    );
    let (hub, remote) = fake_hub(fw, route, "control", refuses_platform);
    let (core, selected) = link_core(&hub);
    let observed = if let Some(notice) =
        notice_of(&core, &selected).filter(|_| selected.is_err() || core.device_protocol().is_err())
    {
        // Long enough for a poll at 10 Hz to show again.
        std::thread::sleep(Duration::from_millis(600));
        let sent = hub.requests();
        let page0 = |r: &&dspi_link::fakehub::DeviceRequest| r.sent_page() == 0;
        let platform = sent
            .iter()
            .filter(page0)
            .filter(|r| r.b_request == op::REQ_GET_PLATFORM)
            .count();
        let meter = sent
            .iter()
            .filter(page0)
            .filter(|r| r.b_request == op::REQ_GET_STATUS && r.w_value == 9)
            .count();
        let what = if sent.is_empty() {
            Sent::Nothing
        } else if sent.len() == 1 && platform == 1 {
            Sent::PlatformOnly
        } else if platform == 1 && meter + platform == sent.len() {
            Sent::PlatformAndMeterPoll
        } else {
            panic!("{fw:?} {route:?}: sent after a notice: {sent:?}")
        };
        notes.connect = format!("{selected:?}");
        notes.extra.push(format!(
            "reached the device: {:?}",
            sent.iter()
                .map(|r| format!("0x{:02X}/{}", r.b_request, r.w_value))
                .collect::<Vec<_>>()
        ));
        // Each page-0 request that reached a paged device is one the fake let
        // through (it refuses it, as stock 1.1.7 does) and counted.
        if refuses_platform {
            let v = u32::try_from(sent.len()).unwrap();
            assert!(remote.violations() >= v, "{fw:?} {route:?}");
            let last = remote.last_violation().unwrap_or_default();
            assert!(last.contains("page-0 request"), "{fw:?} {route:?}: {last}");
        } else {
            no_violation(fw, route, &remote, 0);
        }
        Outcome::Notice { notice, sent: what }
    } else if let Err(e) = selected {
        // The core does not connect (the sender: its vendor subset); what the
        // device is at the transport, through the same hub, as control and admin.
        assert_eq!(fw, Fw::Sender, "{fw:?} {route:?}: select: {e}");
        notes.connect = format!("the core's connect sequence: {e}");
        core.link_close();
        no_violation(fw, route, &remote, 0);
        let (paged, control_ext) = over_link(fw, route, "control");
        let (_, admin_ext) = over_link(fw, route, "admin");
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
        // No request with a page through a 1.2 hub (S1, OD-PGe).
        if route != Route::L13 {
            let paged_sent: Vec<_> = hub
                .requests()
                .into_iter()
                .filter(|r| r.sent_page() != 0)
                .collect();
            assert!(paged_sent.is_empty(), "{fw:?} {route:?}: {paged_sent:?}");
        }
        // The extension channel as admin.
        let (admin_hub, admin_remote) = fake_hub(fw, route, "admin", false);
        let (admin, selected) = link_core(&admin_hub);
        selected.unwrap_or_else(|e| panic!("{fw:?} {route:?}: admin select: {e}"));
        let admin_ext = !admin.ext_status().unwrap().supported.is_empty();
        admin.link_close();
        no_violation(fw, route, &admin_remote, 0);
        notes.ext = format!(
            "control: {}, admin: {}",
            if control_ext { "listed" } else { "empty" },
            if admin_ext { "listed" } else { "empty" }
        );
        let ext = ext_of(control_ext, admin_ext);
        Outcome::Full {
            paged,
            ext,
            boot,
            bulk,
            generator,
        }
    };
    // The plan's accepted gap: a paged Zero behind an upstream 1.3 hub reads as a
    // Pico, so the gated entry sends `SYS.ENTER_BOOTLOADER` (to the fake's foreign
    // loader, a violation it is told to let through here).
    if fw == Fw::ZeroPaged && route == Route::L13 {
        let (hub, remote) = fake_hub(fw, route, "admin", true);
        let (core, selected) = link_core(&hub);
        selected.unwrap();
        let entry =
            core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)));
        notes.extra.push(format!(
            "gated entry as admin: {entry:?}, device went to {:?}, violations {}",
            remote.rebooted_to(),
            remote.violations()
        ));
        assert!(
            remote.rebooted_to().is_some(),
            "the entry went out: {entry:?}"
        );
        core.link_close();
    }
    Cell {
        fw,
        route,
        expected,
        observed,
        notes,
    }
}
// ── The matrix ──────────────────────────────────────────────────────────────

fn run(route: Route) {
    let cells: Vec<Cell> = ROWS
        .iter()
        .map(|&fw| match route {
            Route::Usb | Route::Midi => direct(fw, route),
            _ => through_hub(fw, route),
        })
        .collect();
    check(&format!("core-{}", route.key()), &cells);
}

#[test]
fn wp_pgmat_usb() {
    run(Route::Usb);
}

#[test]
fn wp_pgmat_usb_midi() {
    run(Route::Midi);
}

#[test]
fn wp_pgmat_l12_hub_forwarding_the_page() {
    run(Route::L12Forward);
}

#[test]
fn wp_pgmat_l12_hub_dropping_the_page() {
    run(Route::L12Drop);
}

#[test]
fn wp_pgmat_l13_upstream_hub() {
    run(Route::L13);
}
