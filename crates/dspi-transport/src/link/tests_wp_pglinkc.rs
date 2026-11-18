//! WP PGLINKC: the DSPi Link client for 1.2 and 1.3 hubs (PLAN-PG section 1.4,
//! section 1.1 steps 5, 6 and 8), against the fake hub's `L12Forward`,
//! `L12Drop` and `L13` modes with PGFAKE1's paged fakes behind them.

use std::collections::VecDeque;

use dspi_link::fakehub::{
    Auth, DeviceRequest, FakeHub, HubConfig, HubDevice, HubMode, SharedDevice, req,
};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions};
use dspi_proto::paged::{LastResult, PagedDir, ProbeReply, ProtocolInfo};

use super::*;
use crate::fake::FakeGeneration;
use crate::paged::Generation;
use crate::{FakeDevice, FakeOptions};

const PIN: &str = "482913";
const WAIT: Duration = Duration::from_secs(3);

/// A paged fake (or any fake) behind the fake hub, as a USB hub forwards to
/// it: page 0 as the 1.1.6 calls, any other page as the paged calls. A STALL
/// is status 0x02; with `explain` (a 1.3 hub, spec 1.3 section 9.1) a paged
/// STALL is the `SYS.LAST_RESULT` code when the record names the request.
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
        let page = (w_index >> 8) as u8;
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

/// `generation` as a fake behind a hub; `allow`: let a violation through
/// (a test that sends one on purpose, and counts it).
fn fake(
    generation: FakeGeneration,
    allow: bool,
    explain: bool,
) -> (SharedDevice, crate::FakeRemote) {
    let dev = FakeDevice::new(
        crate::fake::Capture::demo(),
        FakeOptions {
            allow_violation: allow,
            ..generation.options()
        },
    );
    let remote = dev.remote();
    (SharedDevice::new(OnHub { dev, explain }), remote)
}

/// A device that answers from a script: the hub's probe as paged 1.0 (when
/// `paged`), then each request with the next status (OK once it runs out).
struct Scripted {
    paged: bool,
    statuses: VecDeque<u8>,
}

impl HubDevice for Scripted {
    fn request(
        &mut self,
        _: Dir,
        b_request: u8,
        _: u16,
        w_index: u16,
        _: u16,
        _: &[u8],
    ) -> (u8, Vec<u8>) {
        if b_request == 0x10 && w_index == 0x0102 {
            return if self.paged {
                (
                    status::OK,
                    b"DSPi\x01\x00\x0C\x00\x40\x00\x00\x01\0\0\0\0".to_vec(),
                )
            } else {
                (status::ERROR, Vec::new())
            };
        }
        match self.statuses.pop_front() {
            Some(s) => (s, Vec::new()),
            None => (status::OK, vec![1, 2, 3, 4]),
        }
    }
}

fn scripted(paged: bool, statuses: &[u8]) -> SharedDevice {
    SharedDevice::new(Scripted {
        paged,
        statuses: statuses.iter().copied().collect(),
    })
}

fn options() -> ConnectOptions {
    ConnectOptions {
        client_name: "wp-pglinkc".into(),
        app: "dspi-transport tests".into(),
        app_version: "test".into(),
        token: None,
    }
}

/// A session with `role`: paired with the PIN when the hub asks for one.
fn session(hub: &FakeHub, role: &str) -> LinkClient {
    let (c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &options(), None)
        .expect("connect to the fake hub");
    if state != AuthState::Ready {
        c.pair(PIN, role).expect("pair");
    }
    assert!(!c.devices().is_empty(), "the device is listed");
    c
}

/// A hub with PIN auth granting `role`.
fn start(cfg: HubConfig, role: &str) -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        pin_role: role.into(),
        ..cfg
    })
    .unwrap()
}

fn transport(c: &LinkClient) -> LinkTransport {
    let serial = c.devices()[0].serial.clone();
    let t = LinkTransport::new(c.clone(), 0, &serial);
    drop(t.notifications());
    t
}

fn opened(c: &LinkClient) -> Result<Box<dyn Transport>> {
    crate::paged::open(Box::new(transport(c)))
}

fn wait(mut f: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < WAIT {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn paged_requests(hub: &FakeHub) -> Vec<DeviceRequest> {
    hub.requests()
        .into_iter()
        .filter(|r| r.sent_page() != 0)
        .collect()
}

fn notice<T>(r: &Result<T>) -> Option<LinkNotice> {
    r.as_ref().err().and_then(LinkNotice::of)
}

#[test]
fn wp_pglinkc_the_client_says_minor_3_and_the_hub_constant_stays_2() {
    use dspi_link::messages::{CLIENT_PROTO_MINOR, PROTO_MINOR};
    assert_eq!(CLIENT_PROTO_MINOR, 3);
    assert_eq!(PROTO_MINOR, 2, "our hub's minor rises with PGHDEV (R4)");
    let hello = dspi_link::messages::out::hello(&dspi_link::messages::ClientInfo {
        name: "n".into(),
        app: "a".into(),
        version: "v".into(),
    });
    let v: serde_json::Value = serde_json::from_str(&hello).unwrap();
    assert_eq!(v["proto"]["minor"], 3);
}

#[test]
fn wp_pglinkc_the_route_follows_the_hub_minor_and_the_device_object() {
    use dspi_link::messages::{DeviceInfo, HelloHub, Proto};
    let hub = |minor, caps: &[&str]| HelloHub {
        proto: Proto { major: 1, minor },
        caps: caps.iter().map(|c| c.to_string()).collect(),
        ..HelloHub::default()
    };
    let paged = DeviceInfo {
        protocol: Some(Proto { major: 1, minor: 0 }),
        ..DeviceInfo::default()
    };
    let legacy = DeviceInfo::default();
    // A 1.2 hub: legacy whatever the object says (OD-PGe)
    for minor in [0, 1, 2] {
        for d in [&paged, &legacy] {
            assert_eq!(
                LinkRoute::of(&hub(minor, &[]), Some(d)),
                LinkRoute::Legacy { l12: true }
            );
        }
    }
    assert_eq!(
        LinkRoute::of(&hub(3, &[]), Some(&paged)),
        LinkRoute::Paged { major: 1, minor: 0 }
    );
    assert_eq!(
        LinkRoute::of(&hub(3, &[]), Some(&legacy)),
        LinkRoute::Unserved
    );
    assert_eq!(LinkRoute::of(&hub(3, &[]), None), LinkRoute::Unserved);
    assert_eq!(
        LinkRoute::of(&hub(3, &["legacy_devices"]), Some(&legacy)),
        LinkRoute::Legacy { l12: false },
        "our hub forwards to legacy devices (OD-PGg)"
    );
}

/// Section 1.1 step 6 and the done criterion: in both 1.2 modes, with a
/// control-role session, a G2s gets the "needs a 1.3 hub" notice after one
/// `REQ_GET_PLATFORM` and nothing else, and no request with a page leaves.
#[test]
fn wp_pglinkc_through_a_12_hub_a_g2s_gets_the_notice_after_one_platform_read() {
    for mode in [HubMode::L12Forward, HubMode::L12Drop] {
        let (dev, remote) = fake(FakeGeneration::STOCK117, true, false);
        let cfg = match mode {
            HubMode::L12Forward => HubConfig::l12_forward(Some(dev)),
            _ => HubConfig::l12_drop(Some(dev)),
        };
        let hub = start(cfg, "control");
        let c = session(&hub, "control");
        assert_eq!(c.role(), "control");
        let mut t = opened(&c).expect("legacy, with nothing sent");
        assert_eq!(t.generation(), Some(Generation::Legacy));
        assert!(
            hub.requests().is_empty(),
            "{mode:?}: no probe through a 1.2 hub (S1)"
        );
        // What the identity read sends over Link: the platform and the serial
        let answers = t.control_in_batch(&[
            InRead::upto(op::REQ_GET_PLATFORM, 0, 7),
            InRead::exact(op::REQ_GET_SERIAL, 0, 16),
        ]);
        assert!(
            answers
                .iter()
                .all(|a| notice(a) == Some(LinkNotice::Needs13Hub)),
            "{mode:?}: {answers:?}"
        );
        let sent = hub.requests();
        assert_eq!(
            sent.len(),
            1,
            "{mode:?}: one REQ_GET_PLATFORM and nothing else: {sent:?}"
        );
        assert_eq!(
            (sent[0].b_request, sent[0].sent_page()),
            (op::REQ_GET_PLATFORM, 0)
        );
        assert!(paged_requests(&hub).is_empty());
        assert_eq!(
            remote.violations(),
            1,
            "the one page-0 read, which G2s refuses"
        );
        // A single read says the same
        assert_eq!(
            notice(&t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8)),
            Some(LinkNotice::Needs13Hub)
        );
    }
}

/// A G2b through a 1.2 hub is a legacy device (its legacy stack), its
/// extension channel admin-only as on a 1.2 hub today.
#[test]
fn wp_pglinkc_through_a_12_hub_a_g2b_is_legacy_with_ext_admin_only() {
    for mode in [HubMode::L12Forward, HubMode::L12Drop] {
        for (role, ext_ok) in [("control", false), ("admin", true)] {
            let (dev, remote) = fake(FakeGeneration::Bofr117, false, false);
            let cfg = match mode {
                HubMode::L12Forward => HubConfig::l12_forward(Some(dev)),
                _ => HubConfig::l12_drop(Some(dev)),
            };
            let hub = start(cfg, role);
            let c = session(&hub, role);
            let mut t = opened(&c).unwrap();
            assert_eq!(t.generation(), Some(Generation::Legacy));
            let platform = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
            assert!(platform.len() >= 4);
            let ext = t.control_in_upto(op::REQ_EXTENSION, 0, 64);
            assert_eq!(ext.is_ok(), ext_ok, "{mode:?} {role}: {ext:?}");
            if !ext_ok {
                assert!(matches!(ext, Err(TransportError::Denied(_))), "{ext:?}");
            }
            assert!(paged_requests(&hub).is_empty());
            assert_eq!(remote.violations(), 0);
        }
    }
}

/// Section 1.1 step 5: on a 1.3 hub a device without `protocol` gets the
/// notice and no CMD, through `open` and through any call.
#[test]
fn wp_pglinkc_on_a_13_hub_a_legacy_device_gets_the_notice_and_no_cmd() {
    let hub = start(HubConfig::l13(None), "control");
    let c = session(&hub, "control");
    assert!(c.devices()[0].protocol.is_none());
    let r = opened(&c);
    assert_eq!(notice(&r), Some(LinkNotice::HubServesPagedOnly));
    let mut t = transport(&c);
    assert_eq!(t.route(), LinkRoute::Unserved);
    assert_eq!(
        notice(&t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8)),
        Some(LinkNotice::HubServesPagedOnly)
    );
    assert_eq!(
        notice(&t.control_out(req::SET_MASTER_VOLUME, 0, &[0; 4])),
        Some(LinkNotice::HubServesPagedOnly)
    );
    let batch = t.control_in_batch(&[
        InRead::upto(op::REQ_GET_PLATFORM, 0, 8),
        InRead::exact(op::REQ_GET_SERIAL, 0, 16),
    ]);
    assert!(
        batch
            .iter()
            .all(|a| notice(a) == Some(LinkNotice::HubServesPagedOnly))
    );
    assert!(
        hub.requests().is_empty() && hub.log().is_empty(),
        "no CMD: {:?}",
        hub.requests()
    );
    // A 1.3 hub sends `denied` empty: nothing to grey out on a legacy device
    assert!(c.denied_for(&c.devices()[0]).is_empty());
}

/// A paged device on a 1.3 hub: the probe is answered from the device object
/// (nothing sent), every CMD carries `(page << 8) | 2` and a non-zero GET
/// length, and nothing page-0 is sent.
#[test]
fn wp_pglinkc_a_paged_device_gets_paged_cmds_only() {
    let (dev, remote) = fake(FakeGeneration::STOCK117, false, true);
    let hub = start(HubConfig::l13(Some(dev)), "control");
    let c = session(&hub, "control");
    assert_eq!(
        c.devices()[0].protocol.map(|p| (p.major, p.minor)),
        Some((1, 0))
    );
    let mut t = transport(&c);
    assert_eq!(t.route(), LinkRoute::Paged { major: 1, minor: 0 });
    assert!(t.carries_pages());
    let probe = t.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    assert!(
        matches!(ProtocolInfo::decode(&probe), ProbeReply::Paged(i) if i.major == 1 && i.minor == 0)
    );
    assert!(hub.requests().is_empty(), "the probe sent nothing (S1)");
    let platform = t.read_platform().unwrap();
    assert!(platform.len() >= 7, "{platform:?}");
    t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &(-20.0f32).to_le_bytes())
        .unwrap();
    let sent = hub.requests();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        (sent[0].b_request, sent[0].sent_w_index, sent[0].w_length),
        (0x20, 0x0102, PLATFORM_ASK)
    );
    assert_eq!(
        (sent[1].b_request, sent[1].sent_w_index, sent[1].w_length),
        (0x50, 0x0502, 4)
    );
    // A zero-length GET and a 1.1.6 call are refused here, with nothing sent
    assert!(t.control_in_paged(cmds::SYS_PLATFORM, 0, 0).is_err());
    assert!(
        matches!(t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8), Err(TransportError::Incompatible(m)) if m == PAGED_REFUSES_PAGE0)
    );
    assert!(matches!(
        t.control_out(req::SET_MASTER_VOLUME, 0, &[0; 4]),
        Err(TransportError::Incompatible(_))
    ));
    assert!(t.control_in_batch(&[InRead::upto(op::REQ_GET_PLATFORM, 0, 8)])[0].is_err());
    // The chunked bulk form never goes over Link (spec 1.3 section 8.2)
    assert!(
        t.control_out_paged(cmds::STORE_ALL_PARAMS_CHUNK, 0, &[0; 8])
            .is_err()
    );
    assert_eq!(hub.requests().len(), 2);
    assert!(
        hub.requests()
            .iter()
            .all(|r| r.sent_page() != 0 && r.sent_w_index & 0xFF == 2)
    );
    assert_eq!(remote.violations(), 0);
}

/// R3: the probe costs no round trip, and a discovery batch is one window of
/// requests on the wire (all sent before the first answer).
#[test]
fn wp_pglinkc_a_paged_connect_is_the_probe_for_free_and_one_discovery_batch() {
    let (dev, _remote) = fake(FakeGeneration::Bofr117, false, true);
    let hub = start(
        HubConfig {
            max_inflight: 8,
            ..HubConfig::l13(Some(dev))
        },
        "control",
    );
    let c = session(&hub, "control");
    let mut t = transport(&c);
    let r = crate::paged::open(Box::new(transport(&c)));
    // Until WP PGADAPT the paged branch refuses after the probe; either way
    // the probe itself put nothing on the wire
    drop(r);
    assert!(hub.requests().is_empty());
    let discovery = [
        (cmds::SYS_PAGE_LIST, 0, 256),
        (cmds::SYS_PLATFORM, 0, PLATFORM_ASK),
        (cmds::SYS_PAGE_COMMANDS, 0x01, 64),
        (cmds::SYS_PAGE_COMMANDS, 0x02, 64),
        (cmds::SYS_PAGE_COMMANDS, 0x05, 64),
        (cmds::SYS_PAGE_COMMANDS, 0xE0, 64),
    ];
    hub.hold_replies();
    let batch = std::thread::spawn(move || {
        let answers = t.paged_batch(&discovery);
        (t, answers)
    });
    assert!(
        wait(|| hub.requests().len() == discovery.len()),
        "the whole batch before any answer: {:?}",
        hub.requests()
    );
    hub.release_replies();
    let (_t, answers) = batch.join().unwrap();
    assert!(answers.iter().all(Result::is_ok), "{answers:?}");
    assert_eq!(
        hub.requests().len(),
        discovery.len(),
        "one window, no request twice"
    );
    let one_by_one: Vec<String> = {
        let mut t = transport(&c);
        discovery
            .iter()
            .map(|&(id, v, l)| format!("{:?}", t.control_in_paged(id, v, l)))
            .collect()
    };
    let batched: Vec<String> = answers.iter().map(|a| format!("{a:?}")).collect();
    assert_eq!(batched, one_by_one, "a batch answers as single reads do");
}

#[test]
fn wp_pglinkc_denied_ids_grey_out_a_paged_device_and_denied_a_legacy_one() {
    let (dev, _r) = fake(FakeGeneration::STOCK117, false, true);
    let hub = start(HubConfig::l13(Some(dev)), "control");
    let c = session(&hub, "control");
    assert!(c.denied().is_empty(), "a 1.3 hub sends `denied` empty");
    let ids = c.denied_ids();
    assert!(
        ids.contains(&(0x0130, 0)),
        "SYS.ENTER_BOOTLOADER SET is config: {ids:?}"
    );
    let device = c.devices()[0].clone();
    assert!(device.is_paged());
    assert_eq!(c.denied_for(&device), ids);

    let hub = start(HubConfig::l12_forward(None), "control");
    let c = session(&hub, "control");
    assert!(c.denied_ids().is_empty(), "a 1.2 hub has no denied_ids");
    let denied = c.denied();
    assert!(!denied.is_empty());
    let device = c.devices()[0].clone();
    assert!(!device.is_paged());
    assert_eq!(
        c.denied_for(&device),
        denied
            .iter()
            .map(|&(r, d)| (u16::from(r), d))
            .collect::<Vec<_>>()
    );
}

/// Spec 1.3 section 8.1: 0x10..=0x1A are the firmware's refusals, typed and
/// kept with their request; never retried.
#[test]
fn wp_pglinkc_firmware_result_codes_are_typed_refusals() {
    for code in 0x10u8..=0x1A {
        if code == VC_ERR_BUSY {
            continue; // retryable, below
        }
        let hub = FakeHub::start(HubConfig::l13(Some(scripted(true, &[code])))).unwrap();
        let c = session(&hub, "admin");
        let mut t = transport(&c);
        let r = t.control_in_paged(cmds::SYS_PLATFORM, 0, 8);
        assert!(
            matches!(r, Err(TransportError::Stalled { opcode: 0x20 })),
            "0x{code:02X}: {r:?}"
        );
        assert_eq!(t.last_refusal(), Some((cmds::SYS_PLATFORM, code)));
        assert_eq!(hub.requests().len(), 1, "0x{code:02X} is final");
        assert!(
            !t.needs_reattach(),
            "a refusal before any answer is no generation change"
        );
    }
}

/// Spec 1.3 section 8.1: a paged request is resent only on 0x01, 0x04, 0x17
/// and 0x84.
#[test]
fn wp_pglinkc_retries_only_the_four_codes() {
    for code in [
        status::BUSY,
        status::BULK_LOCKED,
        VC_ERR_BUSY,
        status::RATE_LIMITED,
    ] {
        let hub = FakeHub::start(HubConfig::l13(Some(scripted(true, &[code])))).unwrap();
        let c = session(&hub, "admin");
        let mut t = transport(&c);
        assert!(
            t.control_in_paged(cmds::SYS_PLATFORM, 0, 8).is_ok(),
            "0x{code:02X} goes again"
        );
        assert_eq!(hub.requests().len(), 2);
    }
    for code in [
        status::CRC_ERROR,
        status::ERROR,
        status::TIMEOUT,
        status::OVERSIZE,
        0x12,
        status::TOO_LARGE,
    ] {
        let hub = FakeHub::start(HubConfig::l13(Some(scripted(true, &[code])))).unwrap();
        let c = session(&hub, "admin");
        let mut t = transport(&c);
        assert!(
            t.control_in_paged(cmds::SYS_PLATFORM, 0, 8).is_err(),
            "0x{code:02X} is final"
        );
        assert_eq!(hub.requests().len(), 1, "0x{code:02X} is not resent");
    }
    assert!(
        (0u8..=0xFF)
            .filter(|&s| is_retryable_13(s))
            .eq([0x01, 0x04, 0x17, 0x84])
    );
}

/// OD-PGo: a poll is one paged GET; a read whose plan is several requests is
/// refused before `poll.subscribe`, and the client refuses a poll whose page
/// does not fit the device, with nothing sent.
#[test]
fn wp_pglinkc_polls_on_a_paged_device() {
    let pages = PageSet::vendored(|_| true);
    let spec = paged_poll(1, op::REQ_GET_PLATFORM, 0, 7, 10, &pages).unwrap();
    assert_eq!((spec.req, spec.idx, spec.len), (0x20, 0x0102, 7));
    assert!(matches!(
        paged_poll(1, op::REQ_GET_PLATFORM, 0, 0, 10, &pages),
        Err(PollPlanError::ZeroLength { .. })
    ));
    let multi = (0u8..=0xFF).find(|&o| {
        dspi_proto::paged::plan(o, 0, LegacyBody::In { len: 64 }, &pages)
            .is_ok_and(|p| p.single().is_none())
    });
    let multi = multi.expect("some 1.1.6 read needs several paged requests");
    assert!(matches!(
        paged_poll(1, multi, 0, 64, 10, &pages),
        Err(PollPlanError::MultiRequest { opcode }) if opcode == multi
    ));

    let (dev, _r) = fake(FakeGeneration::STOCK117, false, true);
    let hub = start(HubConfig::l13(Some(dev)), "control");
    let c = session(&hub, "control");
    let page0 = PollSpec {
        slot: 1,
        req: op::REQ_GET_PLATFORM,
        val: 0,
        idx: 2,
        len: 7,
        hz: 5,
    };
    let e = c.poll_subscribe(0, &[page0]).unwrap_err();
    assert!(
        matches!(&e, LinkError::Rejected { code, .. } if code == "unsupported"),
        "{e:?}"
    );
    assert!(
        !hub.json_requests()
            .iter()
            .any(|v| v["t"] == "poll.subscribe"),
        "refused before anything was sent"
    );
    let rx = c.polls(0);
    let granted = c.poll_subscribe(0, &[spec]).unwrap();
    assert_eq!(granted.len(), 1);
    assert!(rx.recv_timeout(WAIT).is_ok(), "the paged poll runs");
    assert!(hub.requests().iter().all(|r| r.sent_w_index == 0x0102));

    // And the reverse: a paged poll to a legacy device
    let hub = start(HubConfig::l12_forward(None), "control");
    let c = session(&hub, "control");
    assert!(c.poll_subscribe(0, &[spec]).is_err());
}

/// Section 1.1 step 8: the fake hub swaps the device's generation under the
/// handle; the transport opened on the old one sends nothing more, and a new
/// open follows the new generation.
#[test]
fn wp_pglinkc_a_swapped_generation_makes_the_client_reattach() {
    let (g2s, g2s_remote) = fake(FakeGeneration::STOCK117, false, true);
    let hub = start(HubConfig::l13(Some(g2s)), "control");
    let c = session(&hub, "control");
    let mut t = transport(&c);
    t.control_in_paged(cmds::SYS_PLATFORM, 0, 8).unwrap();
    let before = hub.requests().len();
    // Re-flashed to 1.1.6 (guarded or not, it must get no paged request)
    let (g1b, g1b_remote) = fake(FakeGeneration::Bofr116 { page_guard: false }, false, false);
    hub.swap_device(Some(g1b));
    assert!(wait(|| c.device(0).is_some_and(|d| d.protocol.is_none())));
    let r = t.control_in_paged(cmds::SYS_PLATFORM, 0, 8);
    assert_eq!(notice(&r), Some(LinkNotice::GenerationChanged));
    assert!(t.needs_reattach());
    assert_eq!(
        notice(&t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &[0; 4])),
        Some(LinkNotice::GenerationChanged)
    );
    assert_eq!(
        hub.requests().len(),
        before,
        "no old-generation request reached the new device"
    );
    // Re-attach: the new transport follows the new object (a 1.3 hub serves
    // no legacy device, so it is the notice, still with nothing sent)
    assert_eq!(notice(&opened(&c)), Some(LinkNotice::HubServesPagedOnly));
    assert_eq!(hub.requests().len(), before);
    assert_eq!(g1b_remote.violations(), 0);

    // The reverse: legacy (on a hub that serves it) to paged
    let (g1b, _g1b) = fake(FakeGeneration::Bofr116 { page_guard: true }, false, false);
    let hub = start(
        HubConfig {
            caps: vec!["cmd".into(), "legacy_devices".into()],
            ..HubConfig::l12_forward(Some(g1b))
        },
        "control",
    );
    let c = session(&hub, "control");
    let mut t = transport(&c);
    t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
    let before = hub.requests().len();
    let (g2s, new_remote) = fake(FakeGeneration::STOCK117, true, true);
    hub.swap_device(Some(g2s));
    // A 1.2 hub does not probe, so the object does not change: the device's
    // refusal is the backstop, and our client still sends it page 0 only
    let r = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8);
    assert!(r.is_err());
    assert!(paged_requests(&hub).is_empty());
    assert_eq!(hub.requests().len(), before + 1);
    assert_eq!(
        new_remote.violations(),
        1,
        "the one page-0 read, refused by G2s"
    );
    assert_eq!(g2s_remote.violations(), 0);
}

/// Section 1.1 step 8, R6c: `0x86` or `0x10` from a device that answered a
/// moment ago is a generation change: the transport goes stale, sends
/// nothing more, and does not retry.
#[test]
fn wp_pglinkc_0x86_or_0x10_after_an_answer_is_a_reattach() {
    for code in [status::UNSUPPORTED, VC_ERR_UNKNOWN] {
        // A paged device
        let hub =
            FakeHub::start(HubConfig::l13(Some(scripted(true, &[status::OK, code])))).unwrap();
        let c = session(&hub, "admin");
        let mut t = transport(&c);
        t.control_in_paged(cmds::SYS_PLATFORM, 0, 8).unwrap();
        assert_eq!(
            notice(&t.control_in_paged(cmds::SYS_PLATFORM, 0, 8)),
            Some(LinkNotice::GenerationChanged)
        );
        assert!(t.needs_reattach());
        assert_eq!(
            notice(&t.control_in_paged(cmds::SYS_PLATFORM, 0, 8)),
            Some(LinkNotice::GenerationChanged)
        );
        assert_eq!(
            hub.requests().len(),
            2,
            "0x{code:02X}: no retry, nothing after"
        );

        // A legacy device on a hub that serves it (our hub: a page-0 request
        // to a device that became paged answers 0x10, R5a)
        let hub = FakeHub::start(HubConfig {
            caps: vec!["cmd".into(), "legacy_devices".into()],
            ..HubConfig::l12_forward(Some(scripted(false, &[status::OK, code])))
        })
        .unwrap();
        let c = session(&hub, "admin");
        let mut t = transport(&c);
        t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
        assert_eq!(
            notice(&t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8)),
            Some(LinkNotice::GenerationChanged)
        );
        assert_eq!(
            notice(&t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8)),
            Some(LinkNotice::GenerationChanged)
        );
        assert_eq!(hub.requests().len(), 2);
    }
}

/// R7c: the client reads 8 platform bytes itself, on both generations.
#[test]
fn wp_pglinkc_eight_platform_bytes_are_read_over_link() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let c = session(&hub, "admin");
    let mut t = transport(&c);
    assert_eq!(t.read_platform().unwrap(), dspi_link::fakehub::PLATFORM);
    let sent = hub.requests();
    assert_eq!(
        (sent[0].b_request, sent[0].w_length, sent[0].sent_page()),
        (op::REQ_GET_PLATFORM, 8, 0)
    );

    let (dev, _r) = fake(FakeGeneration::STOCK117, false, true);
    let hub = start(HubConfig::l13(Some(dev)), "control");
    let c = session(&hub, "control");
    let mut t = transport(&c);
    assert!(t.read_platform().is_ok());
    let sent = hub.requests();
    assert_eq!(
        (sent[0].b_request, sent[0].sent_w_index, sent[0].w_length),
        (0x20, 0x0102, 8)
    );
}

/// Every notice has its text in `assets/notices/notices.json`.
#[test]
fn wp_pglinkc_every_link_notice_has_a_notice() {
    let notices = include_str!("../../../../assets/notices/notices.json");
    for n in LinkNotice::ALL {
        assert!(
            notices.contains(&format!("\"transfer.{}\"", n.code())),
            "{n:?}"
        );
        assert_eq!(LinkNotice::of(&n.error()), Some(n));
        assert_eq!(
            crate::paged::Refusal::of(&n.error()),
            None,
            "not one of open's refusals"
        );
    }
}

/// The fake hub's own 1.3 rules (its oracle role in this WP's tests).
#[test]
fn wp_pglinkc_the_fake_13_hub_forwards_nothing_to_a_legacy_device() {
    let hub = FakeHub::start(HubConfig::l13(None)).unwrap();
    let c = session(&hub, "admin");
    let r = c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(r.status, status::UNSUPPORTED);
    let polls = [PollSpec {
        slot: 1,
        req: req::GET_STATUS,
        val: 9,
        idx: 2,
        len: 4,
        hz: 5,
    }];
    assert!(
        matches!(c.poll_subscribe(0, &polls), Err(LinkError::Rejected { code, .. }) if code == "unsupported")
    );
    assert!(hub.requests().is_empty());
}
