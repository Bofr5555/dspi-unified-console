//! WP PGHDEV: our hub serves both protocol generations (PLAN-PG section 1.5).
//!
//! The real hub in-process, with PGFAKE1/2's fakes of every generation (G1s
//! `Stock116`, G1b `Bofr116` with and without PAGEGUARD, G2s `Stock117`, G2b
//! `Bofr117`) and the DSPi-Zero. A fake panics on a violation of PLAN-PG section
//! 1.1's hard rules (a paged request other than the probe to a 1.1.6 device, a page-0
//! request to stock 1.1.7); the worker would swallow that panic, so every test also
//! asserts `violations() == 0` at the end.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::{
    AccessMode, Completion, Hub, HubConfig, NetworkConfig, Outbound, Pop, Role, Session,
};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::messages::{LINK_13_MINOR, PollSpec};
use dspi_link::{Dir, status};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::paged::legacy::{VC_ERR_LENGTH, VC_ERR_UNKNOWN};
use dspi_proto::paged::{CmdId, EXT_TUNNEL, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

const WAIT: Duration = Duration::from_secs(10);
/// `ROUTE.MASTER_VOLUME` and its 1.1.6 forms.
const VOLUME: CmdId = cmds::ROUTE_MASTER_VOLUME;

fn hub() -> Hub {
    Hub::start(HubConfig::in_process("test")).unwrap()
}

fn fake(g: FakeGeneration, serial: &str) -> (FakeDevice, FakeRemote) {
    let dev = FakeDevice::demo_generation(g).with_serial(serial);
    let remote = dev.remote();
    (dev, remote)
}

/// A session whose client said `minor` in its `hello`.
fn session(hub: &Hub, role: Role, minor: u32) -> Arc<Session> {
    let s = hub.open_session(role, None).unwrap();
    s.set_link_minor(minor);
    s
}

fn command(hub: &Hub, s: &Session, req: CmdRequest) -> CmdResponse {
    let (tx, rx) = mpsc::channel();
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    rx.recv_timeout(WAIT).unwrap()
}

/// A request as a 1.3 client sends it: page in the `wIndex` high byte.
fn paged(handle: u8, dir: Dir, id: CmdId, value: u16, payload: Vec<u8>) -> CmdRequest {
    CmdRequest {
        tag: 7,
        handle,
        dir,
        b_request: id.cmd(),
        w_value: value,
        w_index: id.w_index(),
        w_length: match dir {
            Dir::Get => 64,
            Dir::Set => payload.len() as u16,
        },
        payload,
    }
}

/// A 1.1.6-shaped request (page 0, interface 2).
fn legacy(handle: u8, dir: Dir, opcode: u8, value: u16, payload: Vec<u8>, len: u16) -> CmdRequest {
    CmdRequest {
        tag: 9,
        handle,
        dir,
        b_request: opcode,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: if dir == Dir::Get {
            len
        } else {
            payload.len() as u16
        },
        payload,
    }
}

fn db(v: f32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn no_violation(g: FakeGeneration, remote: &FakeRemote) {
    assert_eq!(
        remote.violations(),
        0,
        "{g:?}: {:?}",
        remote.last_violation()
    );
}

/// An L13-style client (minor 3, paged requests): works by page on G2s and G2b;
/// `0x86` for a page toward a 1.1.6 device (G1s, G1b guarded or not), with nothing
/// sent; `0x10` for a page-0 request toward a paged one, with nothing sent.
#[test]
fn wp_pghdev_an_l13_client_works_by_page_on_paged_devices_only() {
    for g in FakeGeneration::ALL {
        let hub = hub();
        let (dev, remote) = fake(g, "PGHDEV-L13");
        let h = hub.attach(Box::new(dev), 0).unwrap();
        let s = session(&hub, Role::Admin, LINK_13_MINOR);
        let probes = remote.paged_requests();

        let set = command(&hub, &s, paged(h, Dir::Set, VOLUME, 0, db(-12.5)));
        let get = command(&hub, &s, paged(h, Dir::Get, VOLUME, 0, Vec::new()));
        let page0 = command(
            &hub,
            &s,
            legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
        );
        if g.is_paged() {
            assert_eq!(set.status, status::OK, "{g:?}");
            assert_eq!((get.status, get.payload), (status::OK, db(-12.5)), "{g:?}");
            assert_eq!(page0.status, VC_ERR_UNKNOWN, "{g:?}: page 0 is 0x10");
        } else {
            assert_eq!(set.status, status::UNSUPPORTED, "{g:?}");
            assert_eq!(get.status, status::UNSUPPORTED, "{g:?}");
            assert_eq!(page0.status, status::OK, "{g:?}: page 0 works");
            assert_eq!(
                remote.paged_requests(),
                probes,
                "{g:?}: a paged request reached a 1.1.6 device"
            );
            assert_eq!(probes, 1, "{g:?}: the attach probe only");
        }
        no_violation(g, &remote);
    }
}

/// An L12-style client (minor 2: upstream 1.1.6 consoles, our older apps, the
/// vendored hub page) works on every generation: as before on 1.1.6, through the
/// hub's translation on 1.1.7, which a 1.3 client then reads back by page.
#[test]
fn wp_pghdev_an_l12_client_works_on_every_generation() {
    for g in FakeGeneration::ALL {
        let hub = hub();
        let (dev, remote) = fake(g, "PGHDEV-L12");
        let h = hub.attach(Box::new(dev), 0).unwrap();
        let old = session(&hub, Role::Admin, 2);
        let set = command(
            &hub,
            &old,
            legacy(h, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, db(-21.0), 0),
        );
        assert_eq!(set.status, status::OK, "{g:?}");
        let get = command(
            &hub,
            &old,
            legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
        );
        assert_eq!((get.status, get.payload), (status::OK, db(-21.0)), "{g:?}");
        // The whole image too (the bulk relay, translated on 1.1.7).
        let image = command(
            &hub,
            &old,
            legacy(h, Dir::Get, op::REQ_GET_ALL_PARAMS, 0, Vec::new(), 8192),
        );
        assert_eq!(image.status, status::OK, "{g:?}");
        assert_eq!(image.payload, remote.image(), "{g:?}");
        if g.is_paged() {
            let new = session(&hub, Role::Admin, LINK_13_MINOR);
            let by_page = command(&hub, &new, paged(h, Dir::Get, VOLUME, 0, Vec::new()));
            assert_eq!(
                by_page.payload,
                db(-21.0),
                "{g:?}: the translation reached it"
            );
            // And the image by page is the same blob.
            let bulk = command(
                &hub,
                &new,
                CmdRequest {
                    w_length: 8192,
                    ..paged(h, Dir::Get, cmds::STORE_ALL_PARAMS, 0, Vec::new())
                },
            );
            assert_eq!((bulk.status, &bulk.payload), (status::OK, &remote.image()));
            // Written back by page: the bulk relay of 0xA1, on 0x0240.
            let back = command(
                &hub,
                &new,
                paged(h, Dir::Set, cmds::STORE_ALL_PARAMS, 0, bulk.payload.clone()),
            );
            assert_eq!(back.status, status::OK, "{g:?}");
            assert_eq!(remote.image(), bulk.payload);
        }
        no_violation(g, &remote);
    }
}

/// Detach serial X as G1b and attach X as G2s, and back: the handle is rebuilt, the
/// generation decided again; no paged request reaches the G1b fake and no page-0
/// request the G2s fake.
#[test]
fn wp_pghdev_a_reattach_as_another_generation_rebuilds_the_handle() {
    let hub = hub();
    let s = session(&hub, Role::Admin, LINK_13_MINOR);
    let old = session(&hub, Role::Admin, 2);
    let g1b = FakeGeneration::Bofr116 { page_guard: false };
    for round in 0..2 {
        let (dev, legacy_remote) = fake(g1b, "PGHDEV-X");
        let h = hub.attach(Box::new(dev), 0).unwrap();
        assert_eq!(
            command(&hub, &s, paged(h, Dir::Get, VOLUME, 0, Vec::new())).status,
            status::UNSUPPORTED,
            "round {round}: G1b"
        );
        assert_eq!(
            command(
                &hub,
                &old,
                legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4)
            )
            .status,
            status::OK
        );
        assert!(hub.devices()[0].identity.protocol.is_none());
        hub.detach(h).unwrap();
        assert_eq!(legacy_remote.paged_requests(), 1, "the probe only");
        no_violation(g1b, &legacy_remote);

        let (dev, paged_remote) = fake(FakeGeneration::STOCK117, "PGHDEV-X");
        let h2 = hub.attach(Box::new(dev), 0).unwrap();
        assert_eq!(h2, h, "the same serial keeps its handle");
        assert_eq!(
            command(&hub, &s, paged(h, Dir::Get, VOLUME, 0, Vec::new())).status,
            status::OK,
            "round {round}: G2s"
        );
        assert_eq!(
            command(
                &hub,
                &s,
                legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4)
            )
            .status,
            VC_ERR_UNKNOWN
        );
        assert_eq!(
            command(
                &hub,
                &old,
                legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4)
            )
            .status,
            status::OK,
            "translated for a 1.2 session"
        );
        let listed = hub.devices().remove(0);
        assert_eq!(listed.identity.protocol, Some((1, 0)));
        assert_eq!(listed.identity.ext, Some(false), "G2s has no page 0xE0");
        hub.detach(h).unwrap();
        no_violation(FakeGeneration::STOCK117, &paged_remote);
    }
}

/// A raw wire whose probe waits for the test: the attach probe is in flight.
struct Gate {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
    /// Told when the probe starts; the probe answers when `release` sends.
    started: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    /// Every OUT request that reached the wire: `(wIndex, bRequest)`.
    log: Arc<Mutex<Vec<(u16, u8)>>>,
}

impl Gate {
    fn note_out(&self, w_index: u16, opcode: u8) {
        self.log.lock().unwrap().push((w_index, opcode));
    }
}

impl Transport for Gate {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.note_out(2, o);
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
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
        true
    }
    fn control_in_paged(&mut self, id: CmdId, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if id == dspi_proto::paged::SYS_PROTOCOL_INFO {
            let _ = self.started.send(());
            let _ = self.release.lock().unwrap().recv_timeout(WAIT);
        }
        self.inner.control_in_paged(id, v, l)
    }
    fn control_out_paged(&mut self, id: CmdId, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.note_out(id.w_index(), id.cmd());
        self.inner.control_out_paged(id, v, d)
    }
}

/// A device is neither listed nor forwardable before its attach probe has finished:
/// a CMD meanwhile is `NO_DEVICE`, and is never sent.
#[test]
fn wp_pghdev_a_cmd_during_the_attach_probe_is_no_device_and_not_sent() {
    for g in [FakeGeneration::STOCK117, FakeGeneration::Stock116] {
        let hub = hub();
        let (inner, remote) = fake(g, "PGHDEV-GATE");
        let (started_tx, started) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let gate = Gate {
            inner,
            descriptor: DeviceDescriptor {
                serial: "PGHDEV-GATE".into(),
                bus_id: "test".into(),
                address: 1,
                // -- PG-C merge -- PGZERO's descriptor fields
                ..Default::default()
            },
            started: started_tx,
            release: Mutex::new(release_rx),
            log: Arc::clone(&log),
        };
        let attaching = {
            let hub = hub.handle();
            std::thread::spawn(move || hub.attach(Box::new(gate), 0))
        };
        started.recv_timeout(WAIT).unwrap();
        assert!(hub.devices().is_empty(), "{g:?}: listed during its probe");
        let old = session(&hub, Role::Admin, 2);
        let new = session(&hub, Role::Admin, LINK_13_MINOR);
        let during = [
            command(
                &hub,
                &old,
                legacy(0, Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, db(-3.0), 0),
            ),
            command(&hub, &new, paged(0, Dir::Set, VOLUME, 0, db(-3.0))),
        ];
        for r in during {
            assert_eq!(r.status, status::NO_DEVICE, "{g:?}");
        }
        release.send(()).unwrap();
        let h = attaching.join().unwrap().unwrap();
        assert_eq!(h, 0);
        // Answered now: anything queued would have run first.
        let after = command(
            &hub,
            &old,
            legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
        );
        assert_eq!(after.status, status::OK);
        assert_ne!(after.payload, db(-3.0), "{g:?}: a refused write landed");
        let seen = log.lock().unwrap().clone();
        assert!(
            !seen.contains(&(2, op::REQ_SET_MASTER_VOLUME))
                && !seen.contains(&(VOLUME.w_index(), VOLUME.cmd())),
            "{g:?}: a write sent during the probe reached the device: {seen:?}"
        );
        no_violation(g, &remote);
    }
}

/// Two polls on the same command number on different pages are two subscriptions,
/// each answered with its own command's reply; a page toward a legacy device and a
/// 1.3 session's page 0 toward a paged one are refused.
#[test]
fn wp_pghdev_polls_are_keyed_by_page() {
    let hub = hub();
    let (dev, remote) = fake(FakeGeneration::STOCK117, "PGHDEV-POLL");
    let h = hub.attach(Box::new(dev), 0).unwrap();
    let s = session(&hub, Role::Viewer, LINK_13_MINOR);
    let poll = |slot: u8, id: CmdId| PollSpec {
        slot,
        req: id.cmd(),
        val: 0,
        idx: id.w_index(),
        len: 4,
        hz: 10,
    };
    // ROUTE.MASTER_VOLUME and INPUT.LG_SOUND_SYNC_ENABLE: command 0x50 on pages 5 and 3.
    let other = CmdId::new(0x03, VOLUME.cmd());
    let granted = hub
        .poll_subscribe(&s, h, &[poll(0, VOLUME), poll(1, other)])
        .unwrap();
    assert_eq!(granted.len(), 2);
    let mut answers = std::collections::BTreeMap::new();
    let start = Instant::now();
    while answers.len() < 2 {
        assert!(start.elapsed() < WAIT, "{answers:?}");
        if let Pop::Item(Outbound::Poll { slot, payload, .. }) =
            s.outbox().pop(Duration::from_millis(100))
        {
            answers.insert(slot, payload);
        }
    }
    let volume = command(
        &hub,
        &session(&hub, Role::Admin, LINK_13_MINOR),
        paged(h, Dir::Get, VOLUME, 0, Vec::new()),
    );
    assert_eq!(answers[&0], volume.payload, "slot 0 is page 5's");
    assert_ne!(answers[&0].len(), 0);
    assert_ne!(answers[&0], answers[&1], "slot 1 is page 3's: another read");
    // A 1.3 session's page-0 poll of the paged device, refused.
    let page0 = PollSpec {
        req: op::REQ_GET_MASTER_VOLUME,
        idx: dspi_proto::VENDOR_INTERFACE,
        ..poll(2, VOLUME)
    };
    let refused = hub.poll_subscribe(&s, h, &[page0]).unwrap_err();
    assert_eq!(refused.code(), "unsupported");
    // A 1.2 session's page-0 poll is translated.
    let old = session(&hub, Role::Viewer, 2);
    assert!(hub.poll_subscribe(&old, h, &[page0]).is_ok());
    no_violation(FakeGeneration::STOCK117, &remote);
}

/// A bootloader entry toward a board without BOOTSEL (the DSPi-Zero, board id 1 in
/// byte 7 of the 8 platform bytes the hub reads) is `DENIED` in both forms, from any
/// session, with nothing sent (the Zero fake panics on one).
#[test]
fn wp_pghdev_a_tunnelled_bootloader_entry_to_the_zero_is_denied() {
    let hub = hub();
    let dev = FakeDevice::zero().with_serial("PGHDEV-ZERO");
    let remote = dev.remote();
    let h = hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(hub.devices()[0].identity.board, 1);
    for minor in [2, LINK_13_MINOR] {
        let s = session(&hub, Role::Admin, minor);
        let f0 = command(
            &hub,
            &s,
            legacy(h, Dir::Get, op::REQ_ENTER_BOOTLOADER, 0, Vec::new(), 1),
        );
        assert_eq!(f0.status, status::DENIED, "0xF0, minor {minor}");
        let e130 = command(
            &hub,
            &s,
            paged(h, Dir::Set, cmds::SYS_ENTER_BOOTLOADER, 0, Vec::new()),
        );
        assert_eq!(e130.status, status::DENIED, "0x0130, minor {minor}");
    }
    assert_eq!(remote.rebooted_to(), None);
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    // The device is still there and answers.
    let s = session(&hub, Role::Admin, 2);
    let r = command(
        &hub,
        &s,
        legacy(h, Dir::Get, op::REQ_GET_MASTER_VOLUME, 0, Vec::new(), 4),
    );
    assert_eq!(r.status, status::OK);
}

/// G2b's extension tunnel `0xE010` is `config` both ways (our private table): admin
/// only, by page and through the translation of `0x00`; on G2s the adapter refuses
/// it locally and the firmware's code (`0x10`) is forwarded verbatim, as is a wrong
/// SET length (`VC_ERR_LENGTH`).
#[test]
fn wp_pghdev_the_g2b_tunnel_is_admin_only_and_refusals_keep_their_code() {
    let hub = hub();
    let (dev, b_remote) = fake(FakeGeneration::Bofr117, "PGHDEV-G2B");
    let b = hub.attach(Box::new(dev), 0).unwrap();
    let (dev, s_remote) = fake(FakeGeneration::STOCK117, "PGHDEV-G2S");
    let st = hub.attach(Box::new(dev), 0).unwrap();
    let tunnel = |h| paged(h, Dir::Get, EXT_TUNNEL, gext::EXT_LIST, Vec::new());
    let zero = |h| {
        legacy(
            h,
            Dir::Get,
            op::REQ_EXTENSION,
            gext::EXT_LIST,
            Vec::new(),
            64,
        )
    };

    let control13 = session(&hub, Role::Control, LINK_13_MINOR);
    let control12 = session(&hub, Role::Control, 2);
    assert_eq!(command(&hub, &control13, tunnel(b)).status, status::DENIED);
    assert_eq!(command(&hub, &control12, zero(b)).status, status::DENIED);

    let admin13 = session(&hub, Role::Admin, LINK_13_MINOR);
    let admin12 = session(&hub, Role::Admin, 2);
    let by_page = command(&hub, &admin13, tunnel(b));
    assert_eq!(by_page.status, status::OK);
    assert!(!by_page.payload.is_empty());
    let by_zero = command(&hub, &admin12, zero(b));
    assert_eq!(
        (by_zero.status, &by_zero.payload),
        (status::OK, &by_page.payload)
    );

    assert_eq!(command(&hub, &admin13, tunnel(st)).status, VC_ERR_UNKNOWN);
    let short = command(&hub, &admin13, paged(st, Dir::Set, VOLUME, 0, vec![0; 3]));
    assert_eq!(short.status, VC_ERR_LENGTH);

    let devices = hub.devices();
    let ext = |serial: &str| {
        devices
            .iter()
            .find(|d| d.identity.serial == serial)
            .and_then(|d| d.identity.ext)
    };
    assert_eq!(ext("PGHDEV-G2B"), Some(true));
    assert_eq!(ext("PGHDEV-G2S"), Some(false));
    no_violation(FakeGeneration::Bofr117, &b_remote);
    no_violation(FakeGeneration::STOCK117, &s_remote);
}

/// The analyser is shared by id on a paged device (spec 7.15): the `MEAS.RTA_STOP`
/// another session depends on is swallowed, answered `OK` with no payload.
#[test]
fn wp_pghdev_the_analyser_is_shared_by_id() {
    let hub = hub();
    let (dev, remote) = fake(FakeGeneration::STOCK117, "PGHDEV-RTA");
    let h = hub.attach(Box::new(dev), 0).unwrap();
    let a = session(&hub, Role::Control, LINK_13_MINOR);
    let b = session(&hub, Role::Control, LINK_13_MINOR);
    let start = paged(h, Dir::Set, cmds::MEAS_RTA_START, 0, Vec::new());
    assert_eq!(command(&hub, &a, start.clone()).status, status::OK);
    assert_eq!(command(&hub, &b, start).status, status::OK);
    // b reads the bands: it watches.
    let bands = command(
        &hub,
        &b,
        paged(h, Dir::Get, cmds::MEAS_RTA_BANDS, 0, Vec::new()),
    );
    assert_eq!(bands.status, status::OK);
    let stop = command(
        &hub,
        &a,
        paged(h, Dir::Set, cmds::MEAS_RTA_STOP, 0, Vec::new()),
    );
    assert_eq!((stop.status, stop.payload), (status::OK, Vec::new()));
    no_violation(FakeGeneration::STOCK117, &remote);
}

/// The judges on a paged device (PG-B review, finding 8): with a limits profile and
/// the installer lock, a Control session's paged read whose 1.1.6 equivalent is a
/// 1.2 `config` row is still a read (gated on the request as sent, `mutates_id`), and
/// a paged write with no 1.1.6 equivalent (`ROUTE.MASTER_VOLUME` at `wValue` 1) is
/// refused `limits_unknown` even for an Admin; a paged write inside the limits passes.
#[test]
fn wp_pghdev_paged_requests_meet_the_limits_and_the_lock() {
    use dspi_hub::limits;
    use dspi_hub::policy::{Class, Policy, judge_view};
    use dspi_proto::paged::registry::COMMANDS;
    use serde_json::{Value, json};

    let hub = hub();
    let (dev, remote) = fake(FakeGeneration::STOCK117, "PGHDEV-LIM");
    let h = hub.attach(Box::new(dev), 0).unwrap();
    let admin = session(&hub, Role::Admin, LINK_13_MINOR);
    let control = session(&hub, Role::Control, LINK_13_MINOR);
    let msg = |s: &Session, m: Value| -> Value {
        serde_json::from_str(&limits::handle_message(&hub.handle(), s, &m).unwrap()).unwrap()
    };
    let profile = json!({"masterVolumeMaxDb": -10.0, "lockedPages": ["eq"]});
    let set = msg(
        &admin,
        json!({"t": "limits.set", "id": 1, "handle": h, "profile": profile}),
    );
    assert_eq!(set["t"], "ok", "{set}");
    let pin = msg(
        &admin,
        json!({"t": "limits.pin", "id": 2, "handle": h, "pin": "2468"}),
    );
    assert_eq!(pin["t"], "ok", "{pin}");
    assert_eq!(
        msg(&control, json!({"t": "limits.get", "id": 3, "handle": h}))["locked"],
        true
    );

    // A paged read the 1.2 table would call a write by its equivalent.
    let policy = Policy::embedded();
    let read_as_config = COMMANDS
        .iter()
        .filter(|c| c.get && c.build.is_none() && policy.classify_id(c.id, Dir::Get) == Class::Read)
        .find(|c| {
            judge_view(&paged(h, Dir::Get, c.id, 0, Vec::new()))
                .is_some_and(|v| policy.classify(v.b_request, v.dir) != Class::Read)
        })
        .expect("a paged read whose 1.1.6 equivalent is classed above read");
    let r = command(
        &hub,
        &control,
        paged(h, Dir::Get, read_as_config.id, 0, Vec::new()),
    );
    assert_ne!(
        r.status,
        status::DENIED,
        "{}: {r:?}",
        read_as_config.full_name()
    );
    assert_ne!(r.status, status::LOCKED);

    let unjudged = command(&hub, &admin, paged(h, Dir::Set, VOLUME, 1, db(-20.0)));
    assert_eq!(unjudged.status, status::DENIED);
    let why: Value = serde_json::from_slice(&unjudged.payload).unwrap();
    assert_eq!(why["refused"], "limits_unknown", "{why}");

    // Judged by its equivalent: above the limit refused, within it taken, for Control.
    let loud = command(&hub, &control, paged(h, Dir::Set, VOLUME, 0, db(-3.0)));
    let why: Value = serde_json::from_slice(&loud.payload).unwrap();
    assert_eq!(
        (loud.status, &why["refused"]),
        (status::DENIED, &json!("limit_exceeded"))
    );
    let quiet = command(&hub, &control, paged(h, Dir::Set, VOLUME, 0, db(-30.0)));
    assert_eq!(quiet.status, status::OK, "{quiet:?}");
    no_violation(FakeGeneration::STOCK117, &remote);
}

// ------------------------------------------------------------- our client

/// A hub on its real server, open access on loopback.
fn hub_on_the_network() -> (Hub, String) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-pghdev").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        ..HubConfig::in_process("test")
    })
    .unwrap();
    let port = hub.network().unwrap().port();
    (hub, format!("ws://127.0.0.1:{port}/dspi/v1"))
}

/// Our own client (`DspiCore` over DSPi Link, PGLINKC's 1.3 client) through our hub
/// works on every generation: the hub says minor 3, `legacy_devices`, and
/// `protocol` for a paged device, so the client speaks each device's own generation
/// through it (a 1.1.6 device by page 0, a 1.1.7 one by page), with no violation.
#[test]
fn wp_pghdev_our_client_works_on_every_generation() {
    let (hub, url) = hub_on_the_network();
    let mut remotes = Vec::new();
    for (i, g) in FakeGeneration::ALL.into_iter().enumerate() {
        let serial = format!("PGHDEV-OUR{i}");
        let (dev, remote) = fake(g, &serial);
        hub.attach(Box::new(dev), 0).unwrap();
        remotes.push((g, serial, remote));
    }
    for (g, serial, remote) in &remotes {
        let core = dspi_core::DspiCore::new();
        core.link_connect(
            url.clone(),
            "wp-pghdev".into(),
            "0".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let d = core
            .link_devices()
            .into_iter()
            .find(|d| &d.serial == serial)
            .unwrap();
        core.link_select_device(d.handle, d.serial).unwrap();
        core.fetch_all().unwrap_or_else(|e| panic!("{g:?}: {e}"));
        core.set_master_volume(-17.0)
            .unwrap_or_else(|e| panic!("{g:?}: {e}"));
        core.fetch_all().unwrap();
        assert_eq!(core.get_state().master_volume_db, -17.0, "{g:?}");
        no_violation(*g, remote);
    }
}

// -- PG-C merge --
/// PGCORE's batch note: the app's in-process path (`LocalTransport`) and a
/// principal's child (`PrincipalTransport`) say the attachment's generation and
/// carry pages only to a paged device. A core on top shows the generation; its
/// 1.1.6 calls still work on every generation (the hub's adapter translates them),
/// and a raw paged read goes verbatim to a paged device and is `Unsupported` with
/// nothing sent toward a 1.1.6 one.
#[test]
fn pgc_merge_local_and_principal_transports_say_the_generation() {
    use dspi_core::{DspiCore, DspiError, ProtocolGeneration};
    use dspi_transport::Generation;
    let hub = hub();
    let h = hub.handle();
    for (i, g) in FakeGeneration::ALL.into_iter().enumerate() {
        let serial = format!("PGC-LOCAL{i}");
        let (dev, remote) = fake(g, &serial);
        hub.attach(Box::new(dev), 0).unwrap();
        let local = h.open_local(&serial).unwrap();
        assert_eq!(local.carries_pages(), g.is_paged(), "{g:?}");
        assert_eq!(
            matches!(local.generation(), Some(Generation::Paged(_))),
            g.is_paged(),
            "{g:?}"
        );
        let parent = h.open_session(Role::Admin, None).unwrap();
        let child = h.open_child(&parent).unwrap();
        let principal = h.open_as(&serial, &child, false).unwrap();
        assert_eq!(principal.carries_pages(), g.is_paged(), "{g:?}");
        assert_eq!(principal.generation(), local.generation(), "{g:?}");
        drop(principal);

        let core = DspiCore::new();
        core.open_transport(Box::new(local), serial.clone(), 0)
            .unwrap_or_else(|e| panic!("{g:?}: {e}"));
        let want = if g.is_paged() {
            ProtocolGeneration::Paged
        } else {
            ProtocolGeneration::Legacy
        };
        assert_eq!(core.device_protocol().unwrap().generation, want, "{g:?}");
        core.set_master_volume(-11.0)
            .unwrap_or_else(|e| panic!("{g:?}: {e}"));
        core.fetch_all().unwrap();
        assert_eq!(core.get_state().master_volume_db, -11.0, "{g:?}");
        let before = remote.paged_requests();
        let raw = core.raw_get_id(VOLUME.0, 0, 4);
        if g.is_paged() {
            let v = raw.unwrap_or_else(|e| panic!("{g:?}: {e}"));
            assert_eq!(v, db(-11.0), "{g:?}");
            assert_eq!(remote.paged_requests(), before + 1, "{g:?}");
        } else {
            assert_eq!(raw, Err(DspiError::Unsupported), "{g:?}");
            assert_eq!(remote.paged_requests(), before, "{g:?}");
        }
        core.disconnect();
        no_violation(g, &remote);
    }
}

// -- review PG-C --
/// A paged DSPi-Zero (stock 1.1.7: a 7-byte `SYS.PLATFORM`, so only its USB
/// descriptor names it) behind our hub is known to the core on top, not only to
/// the hub: the app's in-process path and our client over DSPi Link both get the
/// board the hub's attach read (`DeviceDescriptor::hub_board`, the device
/// object's `board`). So the core names it, offers it no image and refuses the
/// bootloader entry itself, with nothing but the platform read sent; the hub's
/// own `DENIED` stays the backstop.
#[test]
fn review_pgc_a_paged_zero_behind_our_hub_is_known_to_the_core_on_top() {
    use dspi_core::{ConfirmOp, DspiCore, DspiError};
    use dspi_proto::packets::Board;
    use dspi_transport::{Capture, FakeBoard, FakeOptions};
    let (hub, url) = hub_on_the_network();
    let serial = "PGC-PZERO";
    let dev = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            board: FakeBoard::Zero,
            generation: Some(FakeGeneration::STOCK117),
            ..FakeOptions::default()
        },
    )
    .with_serial(serial);
    let remote = dev.remote();
    hub.attach(Box::new(dev), 0).unwrap();
    assert_eq!(hub.devices()[0].identity.board, 1);
    let check = |core: &DspiCore, path: &str| {
        assert_eq!(core.board(), Ok(Board::PiZero2W), "{path}");
        let p = core.device_protocol().unwrap();
        assert_eq!(
            (p.board_id, p.board_name.as_str()),
            (1, "DSPi Zero"),
            "{path}"
        );
        assert!(!core.firmware_target().has_bootsel(), "{path}");
        let entry =
            core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)));
        assert_eq!(entry, Err(DspiError::Unsupported), "{path}");
        assert_eq!(remote.rebooted_to(), None, "{path}");
    };

    // The app's in-process path
    let local = hub.handle().open_local(serial).unwrap();
    assert_eq!(local.descriptor().board(), Some(Board::PiZero2W));
    let core = DspiCore::new();
    core.open_transport(Box::new(local), serial.into(), 0)
        .unwrap();
    check(&core, "local");
    core.disconnect();

    // Our client over DSPi Link
    let core = DspiCore::new();
    core.link_connect(
        url,
        "review-pgc".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let d = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == serial)
        .unwrap();
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    check(&core, "link");
    core.link_close();
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}

// -- review PG-C --
/// A core-voltage raise is refused for a network session, an admin's included,
/// whichever envelope carries it: `REQ_EXTENSION` 0x00 toward a 1.1.6 device, the
/// same toward a paged one (a 1.2 client, translated to the tunnel), and the tunnel
/// `0xE010` by page (a 1.3 client). The stored selection is what it was.
#[test]
fn review_pgc_a_core_voltage_raise_is_denied_through_either_envelope() {
    use dspi_proto::ext::VregOp;
    let hub = hub();
    let raise = VregOp::Set(0x0F).encode().to_vec();
    let generations = [
        FakeGeneration::Bofr116 { page_guard: true },
        FakeGeneration::Bofr117,
    ];
    for (i, g) in generations.into_iter().enumerate() {
        let (dev, remote) = fake(g, &format!("PGC-VREG{i}"));
        let h = hub.attach(Box::new(dev), 0).unwrap();
        let old = session(&hub, Role::Admin, 2);
        let status_of = |hub: &Hub| {
            let r = command(
                hub,
                &old,
                legacy(
                    h,
                    Dir::Get,
                    op::REQ_EXTENSION,
                    gext::EXT_CORE_VREG,
                    Vec::new(),
                    64,
                ),
            );
            assert_eq!(r.status, status::OK, "{g:?}");
            r.payload
        };
        let before = status_of(&hub);
        let r = command(
            &hub,
            &old,
            legacy(
                h,
                Dir::Set,
                op::REQ_EXTENSION,
                gext::EXT_CORE_VREG,
                raise.clone(),
                0,
            ),
        );
        assert_eq!(r.status, status::DENIED, "{g:?}: 0x00");
        if g.is_paged() {
            let new = session(&hub, Role::Admin, LINK_13_MINOR);
            let r = command(
                &hub,
                &new,
                paged(h, Dir::Set, EXT_TUNNEL, gext::EXT_CORE_VREG, raise.clone()),
            );
            assert_eq!(r.status, status::DENIED, "{g:?}: 0xE010");
        }
        assert_eq!(status_of(&hub), before, "{g:?}: nothing was written");
        no_violation(g, &remote);
    }
}
