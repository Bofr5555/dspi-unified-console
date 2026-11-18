//! WP PGWIRE: the probe, the generation wrapper and the fakes' page rule.

use super::*;
use crate::fake::{Capture, FakeOptions};
use crate::mock::{Direction, Exchange, Reply};
use crate::{FakeDevice, MockTransport};

/// The migration guide's own probe reply (section 4.2): `DSPi`, 1.0, 12
/// pages, 64-byte SETs, 256-byte GETs.
const GUIDE_REPLY: [u8; 16] = [
    0x44, 0x53, 0x50, 0x69, 0x01, 0x00, 0x0C, 0x00, 0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0,
];

/// The guide's probe SETUP (section 4.2): `C1 10 00 00 02 01 10 00`.
const GUIDE_SETUP: [u8; 8] = [0xC1, 0x10, 0x00, 0x00, 0x02, 0x01, 0x10, 0x00];

/// A legacy `GET_PLATFORM` reply (RP2350, 1.1.6).
const PLATFORM: [u8; 7] = [1, 1, 0x16, 9, 1, 6, 4];

fn usb_mock() -> MockTransport {
    MockTransport::new().carrying_pages()
}

fn refusal_of(r: Result<Box<dyn Transport>>) -> Refusal {
    match r {
        Err(e) => Refusal::of(&e).unwrap_or_else(|| panic!("not a refusal: {e:?}")),
        Ok(_) => panic!("opened"),
    }
}

#[test]
fn wp_pgwire_the_probe_goes_first_with_the_guides_setup_bytes() {
    let t = usb_mock().data(op::REQ_GET_PLATFORM, PLATFORM.to_vec());
    let log = t.log_handle();
    let g = open(Box::new(t)).unwrap();
    let log = log.lock().unwrap().clone();
    assert_eq!(log[0].setup(), GUIDE_SETUP, "{log:?}");
    assert_eq!(log[0].page(), 0x01);
    // Then GET_PLATFORM on page 0, 8 bytes asked, and nothing else.
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(
        log[1],
        Exchange {
            direction: Direction::In,
            opcode: op::REQ_GET_PLATFORM,
            value: 0,
            payload: Vec::new(),
            index: 2,
            length: PLATFORM_ASK,
        }
    );
    assert_eq!(g.generation(), Some(Generation::Legacy));
}

// -- WP PGADAPT -- a paged device is no longer refused after the probe: it
// gets the discovery, and only paged requests. One that answers the probe
// and nothing else is unrecognised, with no page-0 request sent.
#[test]
fn wp_pgwire_a_paged_device_is_refused_with_nothing_after_the_probe() {
    let t = usb_mock()
        .paged(SYS_PROTOCOL_INFO, Reply::Data(GUIDE_REPLY.to_vec()))
        .answering_everything(vec![0; 64]);
    let log = t.log_handle();
    assert_eq!(refusal_of(open(Box::new(t))), Refusal::Unrecognised);
    let log = log.lock().unwrap().clone();
    assert_eq!(log[0].setup(), GUIDE_SETUP);
    assert!(log.len() > 1, "the discovery follows: {log:?}");
    assert!(
        log.iter().all(|e| e.page() != 0),
        "no page-0 request: {log:?}"
    );
}

#[test]
fn wp_pgwire_a_newer_major_or_a_garbled_answer_sends_nothing_more() {
    let mut newer = GUIDE_REPLY;
    newer[4] = 2;
    let mut wrong_magic = GUIDE_REPLY;
    wrong_magic[0] = b'X';
    for (reply, want) in [
        (newer.to_vec(), Refusal::NewerProtocol),
        (wrong_magic.to_vec(), Refusal::Unrecognised),
        (GUIDE_REPLY[..6].to_vec(), Refusal::Unrecognised),
        (Vec::new(), Refusal::Unrecognised),
    ] {
        let t = usb_mock()
            .paged(SYS_PROTOCOL_INFO, Reply::Data(reply.clone()))
            .data(op::REQ_GET_PLATFORM, PLATFORM.to_vec());
        let log = t.log_handle();
        assert_eq!(refusal_of(open(Box::new(t))), want, "{reply:02X?}");
        assert_eq!(log.lock().unwrap().len(), 1, "{reply:02X?}");
    }
}

#[test]
fn wp_pgwire_a_probe_timeout_is_retried_three_times_then_fails_never_legacy() {
    let t = usb_mock()
        .paged(
            SYS_PROTOCOL_INFO,
            Reply::Error(TransportError::TimedOut {
                opcode: 0x10,
                retries: 0,
            }),
        )
        .data(op::REQ_GET_PLATFORM, PLATFORM.to_vec());
    let log = t.log_handle();
    let started = std::time::Instant::now();
    let r = open(Box::new(t));
    assert!(
        matches!(
            r,
            Err(TransportError::TimedOut {
                opcode: 0x10,
                retries: 3
            })
        ),
        "{:?}",
        r.err()
    );
    let log = log.lock().unwrap().clone();
    assert_eq!(log.len(), 4, "one probe and three retries, no GET_PLATFORM");
    assert!(log.iter().all(|e| e.setup() == GUIDE_SETUP));
    assert!(started.elapsed() >= PROBE_BACKOFF * 3);
}

#[test]
fn wp_pgwire_a_timeout_then_an_answer_decides() {
    let t = usb_mock()
        .paged(
            SYS_PROTOCOL_INFO,
            Reply::Sequence(vec![
                Reply::Error(TransportError::TimedOut {
                    opcode: 0x10,
                    retries: 0,
                }),
                Reply::Stall,
            ]),
        )
        .data(op::REQ_GET_PLATFORM, PLATFORM.to_vec());
    let log = t.log_handle();
    let g = open(Box::new(t)).unwrap();
    assert_eq!(g.generation(), Some(Generation::Legacy));
    assert_eq!(log.lock().unwrap().len(), 3);
}

#[test]
fn wp_pgwire_other_probe_errors_fail_the_open() {
    let t = usb_mock().paged(SYS_PROTOCOL_INFO, Reply::Disconnect);
    let log = t.log_handle();
    assert!(matches!(
        open(Box::new(t)),
        Err(TransportError::Disconnected)
    ));
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn wp_pgwire_a_device_that_refuses_both_is_unrecognised() {
    // Over MIDI a stall is final: no busy retries of GET_PLATFORM.
    struct Midi(MockTransport);
    impl Transport for Midi {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.0.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.0.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> Result<()> {
            self.0.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn caps(&self) -> TransportCaps {
            TransportCaps {
                kind: crate::TransportKind::Midi,
                chunked_bulk: false,
                remote: false,
            }
        }
        fn carries_pages(&self) -> bool {
            true
        }
        fn control_in_paged(&mut self, id: CmdId, v: u16, l: u16) -> Result<Vec<u8>> {
            self.0.control_in_paged(id, v, l)
        }
    }
    let t = usb_mock();
    let log = t.log_handle();
    assert_eq!(refusal_of(open(Box::new(Midi(t)))), Refusal::Unrecognised);
    let log = log.lock().unwrap().clone();
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[1].opcode, op::REQ_GET_PLATFORM);
}

#[test]
fn wp_pgwire_a_link_without_pages_is_legacy_with_nothing_sent() {
    let t = MockTransport::new().answering_everything(vec![0; 64]);
    let log = t.log_handle();
    let g = open(Box::new(t)).unwrap();
    assert_eq!(g.generation(), Some(Generation::Legacy));
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn wp_pgwire_legacy_guard_refuses_every_paged_call_with_nothing_on_the_wire() {
    let t = usb_mock()
        .data(op::REQ_GET_PLATFORM, PLATFORM.to_vec())
        .answering_everything(vec![7; 64]);
    let log = t.log_handle();
    let mut g = open(Box::new(t)).unwrap();
    let before = log.lock().unwrap().len();
    assert!(!g.carries_pages());
    for id in [
        SYS_PROTOCOL_INFO,
        CmdId(0x0553),
        dspi_proto::paged::EXT_TUNNEL,
        CmdId(0x0010),
    ] {
        assert!(
            matches!(g.control_in_paged(id, 0, 16), Err(TransportError::Incompatible(m)) if m == LEGACY_REFUSES_PAGES)
        );
        assert!(matches!(
            g.control_out_paged(id, 0, &[1]),
            Err(TransportError::Incompatible(_))
        ));
    }
    assert_eq!(
        log.lock().unwrap().len(),
        before,
        "nothing reached the wire"
    );
    // Page-0 calls pass, with the raw wire's caps, descriptor and batching.
    assert_eq!(g.control_in(0x50, 0, 4).unwrap(), vec![7; 4]);
    assert_eq!(g.caps(), TransportCaps::USB);
    assert_eq!(g.descriptor().serial, "MOCK000000000000");
    let batch = g.control_in_batch(&[InRead::exact(0x51, 0, 2), InRead::upto(0x52, 0, 3)]);
    assert_eq!(batch.len(), 2);
    let log = log.lock().unwrap().clone();
    assert!(log[before..].iter().all(|e| e.index == 2 && e.page() == 0));
}

/// G1s, G1b without the guard, G1b with PAGEGUARD: legacy after exactly one
/// probe request, through `open`.
#[test]
fn wp_pgwire_every_legacy_fake_is_legacy_after_one_probe_request() {
    for (extensions, page_guard) in [(false, false), (true, false), (true, true)] {
        let dev = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions,
                page_guard,
                ..FakeOptions::default()
            },
        );
        let remote = dev.remote();
        let g = open(Box::new(dev)).unwrap();
        assert_eq!(
            g.generation(),
            Some(Generation::Legacy),
            "ext {extensions} guard {page_guard}"
        );
        assert_eq!(
            remote.paged_requests(),
            1,
            "ext {extensions} guard {page_guard}"
        );
    }
    let demo = FakeDevice::demo();
    let remote = demo.remote();
    assert_eq!(
        open(Box::new(demo)).unwrap().generation(),
        Some(Generation::Legacy)
    );
    assert_eq!(remote.paged_requests(), 1);
}

#[test]
#[should_panic(expected = "reached a 1.1.6-generation device")]
fn wp_pgwire_the_legacy_fake_fails_a_test_that_sends_a_page() {
    let mut dev = FakeDevice::demo();
    // ROUTE.MASTER_VOLUME_SAVED GET: FACTORY_RESET (0x53) on 1.1.6.
    let _ = dev.control_in_paged(CmdId(0x0553), 0, 4);
}

#[test]
fn wp_pgwire_allowed_the_legacy_fake_runs_the_same_number_or_its_guard_stalls() {
    let options = |page_guard| FakeOptions {
        extensions: true,
        page_guard,
        allow_violation: true,
        ..FakeOptions::default()
    };
    // Without the guard 1.1.6 ignores the page: GET_PLATFORM's number on
    // page 5 answers as GET_PLATFORM.
    let mut open_dev = FakeDevice::new(Capture::demo(), options(false));
    let plain = open_dev
        .control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_ASK)
        .unwrap();
    assert_eq!(
        open_dev
            .control_in_paged(CmdId::new(5, op::REQ_GET_PLATFORM), 0, PLATFORM_ASK)
            .unwrap(),
        plain
    );
    // PAGEGUARD stalls any page, both ways.
    let mut guarded = FakeDevice::new(Capture::demo(), options(true));
    let remote = guarded.remote();
    assert!(matches!(
        guarded.control_in_paged(CmdId::new(5, op::REQ_GET_PLATFORM), 0, 8),
        Err(TransportError::Stalled { .. })
    ));
    assert!(matches!(
        guarded.control_out_paged(CmdId(0x0844), 0, &[1]),
        Err(TransportError::Stalled { .. })
    ));
    assert_eq!(remote.paged_requests(), 2);
}

#[test]
fn wp_pgwire_refusals_round_trip_their_words() {
    for r in Refusal::ALL {
        assert_eq!(Refusal::of(&r.error()), Some(r));
        assert_eq!(Refusal::of_text(&r.error().to_string()), Some(r));
    }
    assert_eq!(
        Refusal::of(&TransportError::Incompatible("other".into())),
        None
    );
    assert_eq!(Refusal::of(&TransportError::Disconnected), None);
}
