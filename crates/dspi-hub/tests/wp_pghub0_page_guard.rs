//! WP PGHUB0: our hub refuses a page toward legacy devices.
//!
//! Firmware 1.1.6 ignores `wIndex`, so a paged request (page in the `wIndex` high
//! byte, firmware 1.1.7) would run the 1.1.6 opcode with the same number: GET
//! `0x0553` is `FACTORY_RESET`, SET `0x0844` is `SET_PREAMP` (migration guide 4.1).
//! Every device the hub attaches is of that generation, so the hub answers such a
//! CMD `0x86 UNSUPPORTED` and such a poll `unsupported`, and nothing reaches the
//! device. Page 0 (`wIndex` 0 from the Mac, 2 elsewhere) works as before.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_hub::{Completion, Hub, HubConfig, PollRefused, Role, Session};
use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::messages::PollSpec;
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

const WAIT: Duration = Duration::from_secs(10);

/// The 15 reads of the migration guide 4.1 that each run a 1.1.6 write-as-read,
/// as the 16-bit command id `(page << 8) | command`.
const DANGEROUS_READS: [u16; 15] = [
    0x0553, // ROUTE.MASTER_VOLUME_SAVED -> FACTORY_RESET
    0x0892, // FX.UPMIX_SURROUND_MODE -> PRESET_DELETE
    0x0891, // FX.UPMIX_CENTER_MODE -> PRESET_LOAD
    0x0890, // FX.UPMIX_ENABLE -> PRESET_SAVE
    0x0551, // ROUTE.MASTER_VOLUME_MODE -> SAVE_PARAMS
    0x0351, // INPUT.LG_SOUND_SYNC_STATUS -> SAVE_PARAMS
    0x0A51, // CS.MACRO_HEADER -> SAVE_PARAMS
    0x0A52, // CS.MACRO_STEP -> SAVE_OUTPUT_CONFIG
    0x089D, // FX.UPMIX_PRESENCE -> CS_SAVE
    0x0825, // FX.SUBHARM_MASK -> CS_MACRO_FIRE
    0x0725, // DYN.LEVELLER_GATE -> CS_MACRO_FIRE
    0x0925, // MEAS.RTA_STATUS -> CS_MACRO_FIRE
    0x0868, // FX.TUBE_RECTIFIER -> SET_ADAT_INPUT_ENABLE
    0x086A, // FX.TUBE_XFMR_DAMPING -> SET_ADAT_INPUT_PIN
    0x086C, // FX.TUBE_MIX -> SET_ADAT_INPUT_CLOCK_MODE
];

/// FX.PSYBASS_CHARACTER SET, the old `SET_PREAMP` (+50 dB on every input).
const PSYBASS_CHARACTER: u16 = 0x0844;

/// A demo device that logs every request that reaches it.
struct Counting {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
    log: Arc<Mutex<Vec<u8>>>,
}

impl Counting {
    fn new() -> (Self, FakeRemote, Arc<Mutex<Vec<u8>>>) {
        let inner = FakeDevice::demo();
        let remote = inner.remote();
        let log = Arc::new(Mutex::new(Vec::new()));
        let dev = Self {
            inner,
            descriptor: DeviceDescriptor {
                serial: "PGHUB0".into(),
                bus_id: "test".into(),
                address: 1,
            },
            log: Arc::clone(&log),
        };
        (dev, remote, log)
    }
    fn note(&self, opcode: u8) {
        self.log.lock().unwrap().push(opcode);
    }
}

impl Transport for Counting {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(o);
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(o);
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.note(o);
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
}

fn hub() -> Hub {
    Hub::start(HubConfig::in_process("test")).unwrap()
}

fn command(hub: &Hub, s: &Session, req: CmdRequest) -> CmdResponse {
    let (tx, rx) = std::sync::mpsc::channel();
    hub.submit(s, req, Completion::new(1, move |r| tx.send(r).unwrap()));
    rx.recv_timeout(WAIT).unwrap()
}

/// A request as a 1.1.7 client sends it: page in the high byte, interface 2 low.
fn paged(handle: u8, dir: Dir, id: u16, payload: Vec<u8>) -> CmdRequest {
    let w_length = match dir {
        Dir::Get => 64,
        Dir::Set => payload.len() as u16,
    };
    CmdRequest {
        tag: 7,
        handle,
        dir,
        b_request: (id & 0xFF) as u8,
        w_value: 0,
        w_index: (id & 0xFF00) | dspi_proto::VENDOR_INTERFACE,
        w_length,
        payload,
    }
}

/// The old opcodes the paged ids collide with.
fn colliding() -> Vec<u8> {
    DANGEROUS_READS
        .iter()
        .chain([&PSYBASS_CHARACTER])
        .map(|id| (id & 0xFF) as u8)
        .collect()
}

#[test]
fn wp_pghub0_dangerous_paged_requests_are_refused_and_reach_nothing() {
    let hub = hub();
    let (dev, remote, log) = Counting::new();
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = hub.open_session(Role::Admin, None).unwrap();
    let before = remote.image();

    let mut requests: Vec<CmdRequest> = DANGEROUS_READS
        .iter()
        .map(|id| paged(handle, Dir::Get, *id, Vec::new()))
        .collect();
    requests.push(paged(
        handle,
        Dir::Set,
        PSYBASS_CHARACTER,
        50.0f32.to_le_bytes().to_vec(),
    ));
    // The extreme the old test used, and a page with interface 0 in the low byte.
    requests.push(CmdRequest {
        w_index: 0xFFFF,
        ..paged(handle, Dir::Get, 0x0553, Vec::new())
    });
    requests.push(CmdRequest {
        w_index: 0x0500,
        ..paged(handle, Dir::Get, 0x0553, Vec::new())
    });
    assert_eq!(requests.len(), 18);

    for req in requests {
        let (id, w_index) = (req.b_request, req.w_index);
        let r = command(&hub, &s, req);
        assert_eq!(
            r.status,
            status::UNSUPPORTED,
            "bRequest {id:#04x} wIndex {w_index:#06x}"
        );
        assert!(r.payload.is_empty());
    }
    // A page-0 command after them is answered, so the refused ones are long done.
    let r = command(
        &hub,
        &s,
        CmdRequest {
            w_index: 0,
            ..paged(
                handle,
                Dir::Get,
                u16::from(op::REQ_GET_MASTER_VOLUME),
                Vec::new(),
            )
        },
    );
    assert_eq!(r.status, status::OK);

    let seen = log.lock().unwrap().clone();
    for opcode in colliding() {
        assert!(
            !seen.contains(&opcode),
            "opcode {opcode:#04x} reached the device: {seen:?}"
        );
    }
    assert_eq!(remote.image(), before, "a paged request changed the device");
}

/// The Mac sends `WINDEX_GLOBAL` 0 (`NetworkTransport.swift`), other clients 2: both
/// are page 0 and keep working.
#[test]
fn wp_pghub0_page_zero_from_a_mac_style_client_still_works() {
    let hub = hub();
    let (dev, remote, log) = Counting::new();
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = hub.open_session(Role::Admin, None).unwrap();
    for (w_index, db) in [(0u16, -17.5f32), (dspi_proto::VENDOR_INTERFACE, -9.0)] {
        let volume = db.to_le_bytes().to_vec();
        let write = CmdRequest {
            w_index,
            w_length: 4,
            ..paged(
                handle,
                Dir::Set,
                u16::from(op::REQ_SET_MASTER_VOLUME),
                volume.clone(),
            )
        };
        assert_eq!(
            command(&hub, &s, write).status,
            status::OK,
            "wIndex {w_index}"
        );
        let read = CmdRequest {
            w_index,
            w_length: 4,
            ..paged(
                handle,
                Dir::Get,
                u16::from(op::REQ_GET_MASTER_VOLUME),
                Vec::new(),
            )
        };
        let r = command(&hub, &s, read);
        assert_eq!(r.status, status::OK);
        assert_eq!(r.payload, volume);
        assert!(remote.image().windows(4).any(|w| w == volume));
    }
    assert!(log.lock().unwrap().contains(&op::REQ_SET_MASTER_VOLUME));
}

#[test]
fn wp_pghub0_a_poll_on_a_page_is_refused() {
    let hub = hub();
    let (dev, _remote, _log) = Counting::new();
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let s = hub.open_session(Role::Admin, None).unwrap();
    let page5 = PollSpec {
        slot: 0,
        req: 0x53, // ROUTE.MASTER_VOLUME_SAVED on page 0x05: FACTORY_RESET on 1.1.6
        val: 0,
        idx: 0x0502,
        len: 4,
        hz: 1,
    };
    let refused = hub.poll_subscribe(&s, handle, &[page5]);
    assert_eq!(refused, Err(PollRefused::Unsupported));
    assert_eq!(PollRefused::Unsupported.code(), "unsupported");
    // A page-0 read of the same shape is still taken.
    let meter = PollSpec {
        req: op::REQ_GET_STATUS,
        val: 9,
        idx: dspi_proto::VENDOR_INTERFACE,
        len: 41,
        ..page5
    };
    assert!(hub.poll_subscribe(&s, handle, &[meter]).is_ok());
    let mac = PollSpec { idx: 0, ..meter };
    assert!(hub.poll_subscribe(&s, handle, &[mac]).is_ok());
}
