//! WP PGCORE: the core's DSPi Link attach on paged firmware (PLAN-PG section
//! 1.1 steps 5 and 8, section 1.4, R7c): the side channel's meter poll and
//! background reads are planned with the connection's discovery (no page-0
//! request reaches a paged device), the bind's identity is the device's own
//! 8 platform bytes, and a generation change re-attaches with a new
//! `LinkTransport` and a new `paged::open`.

use std::collections::VecDeque;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig, HubDevice, SharedDevice};
use dspi_link::socket::TcpConnector;
use dspi_link::{ConnectOptions, status};
use dspi_proto::paged::{LastResult, PagedDir, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::link::LinkNotice;
use dspi_transport::{CmdId, FakeDevice, FakeOptions, FakeRemote};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A fake behind the fake hub as a USB hub forwards to it: page 0 as the
/// 1.1.6 calls, a page as the paged calls; a paged STALL is the
/// `SYS.LAST_RESULT` code when the record names the request (a 1.3 hub,
/// spec 1.3 section 9.1).
struct OnHub {
    dev: FakeDevice,
    /// Read `SYS.LAST_RESULT` after a paged STALL: on paged firmware only (a
    /// 1.1.6 device STALLs the hub's probe, and must get nothing more).
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

fn fake(generation: FakeGeneration) -> (SharedDevice, FakeRemote) {
    let dev = FakeDevice::new(
        dspi_transport::fake::Capture::demo(),
        FakeOptions {
            ..generation.options()
        },
    );
    let remote = dev.remote();
    let explain = generation.is_paged();
    (SharedDevice::new(OnHub { dev, explain }), remote)
}

/// An upstream 1.3 hub, open access, in front of `device`.
fn l13(device: SharedDevice) -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::l13(Some(device))
    })
    .unwrap()
}

fn connect(hub: &FakeHub) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect_with(
        hub.url(),
        "wp-pgcore".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    core
}

fn select(core: &DspiCore) -> Result<()> {
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial)
}

/// What reached the device on page 0.
fn page_0(hub: &FakeHub) -> Vec<dspi_link::fakehub::DeviceRequest> {
    hub.requests()
        .into_iter()
        .filter(|r| r.sent_page() == 0)
        .collect()
}

#[test]
fn wp_pgcore_a_paged_device_behind_a_13_hub_gets_no_page_0_request() {
    for generation in [FakeGeneration::STOCK117, FakeGeneration::Bofr117] {
        let (dev, remote) = fake(generation);
        let hub = l13(dev);
        let core = connect(&hub);
        select(&core).unwrap();
        // The background completion runs through the adapter too
        core.ensure_probed();
        core.fetch_all().unwrap();
        let p = core.device_protocol().unwrap();
        assert_eq!(p.generation, ProtocolGeneration::Paged, "{generation:?}");
        assert!(!p.pages.is_empty());
        // The meter poll is the paged GET of the read's plan, and its frames
        // are fed through the same plan into the 1.1.6 block
        let mut live = None;
        wait_until("a live meter block", || {
            live = core.fetch_status().ok().filter(|s| !s.stale);
            live.is_some()
        });
        let n = usize::from(core.get_state().num_channels);
        assert!(live.unwrap().peaks.len() >= n);
        assert!(
            page_0(&hub).is_empty(),
            "{generation:?}: {:?}",
            page_0(&hub)
        );
        assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
        core.link_close();
    }
}

#[test]
fn wp_pgcore_a_generation_change_re_attaches_with_a_new_open() {
    let (g2s, old) = fake(FakeGeneration::STOCK117);
    let hub = l13(g2s);
    let core = connect(&hub);
    select(&core).unwrap();
    core.ensure_probed();
    let id = cmds::ROUTE_MASTER_VOLUME.0;
    assert!(core.raw_get_id(id, 0, 4).is_ok());
    let first = core.connection_generation();
    // Re-flashed to 1.1.6 under the handle: a 1.3 hub serves it nothing, so
    // the re-attach ends in that notice, with nothing sent to the new device
    let (g1b, new) = fake(FakeGeneration::Bofr116 { page_guard: true });
    let before = hub.requests().len();
    hub.swap_device(Some(g1b));
    wait_until("the re-attach's notice", || {
        core.link_status()
            .close_reason
            .contains(LinkNotice::HubServesPagedOnly.text())
    });
    assert_ne!(core.link_status().phase, LinkPhase::Bound);
    assert_eq!(
        hub.requests().len(),
        before,
        "nothing reached the new device"
    );
    // The one paged request it saw is the 1.3 hub's own probe at attach
    assert_eq!(new.paged_requests(), 1);
    // And back to 1.1.7: a new transport, a new probe, a new connection
    let (g2b, newer) = fake(FakeGeneration::Bofr117);
    hub.swap_device(Some(g2b));
    wait_until("bound again", || {
        core.link_status().phase == LinkPhase::Bound && core.connection_generation() != first
    });
    core.ensure_probed();
    let p = core.device_protocol().unwrap();
    assert_eq!(p.generation, ProtocolGeneration::Paged);
    assert_eq!(p.ext_envelope, ExtEnvelope::Tunnel, "the new device's own");
    assert!(core.raw_get_id(id, 0, 4).is_ok());
    assert!(page_0(&hub).is_empty(), "{:?}", page_0(&hub));
    for r in [&old, &new, &newer] {
        assert_eq!(r.violations(), 0, "{:?}", r.last_violation());
    }
    core.link_close();
}

/// A device that answers the hub's probe as paged 1.0, then each request
/// with the next status.
struct Scripted {
    statuses: VecDeque<u8>,
}

impl HubDevice for Scripted {
    fn request(&mut self, _: Dir, b: u8, _: u16, w_index: u16, _: u16, _: &[u8]) -> (u8, Vec<u8>) {
        if b == 0x10 && w_index == 0x0102 {
            return (
                status::OK,
                b"DSPi\x01\x00\x0C\x00\x40\x00\x00\x01\0\0\0\0".to_vec(),
            );
        }
        match self.statuses.pop_front() {
            Some(s) => (s, Vec::new()),
            None => (status::OK, vec![1, 2, 3, 4]),
        }
    }
}

/// `LinkTransport::needs_reattach` reaches the supervisor at once, once.
#[test]
fn wp_pgcore_the_watched_wire_tells_the_supervisor_of_a_generation_change() {
    let hub = l13(SharedDevice::new(Scripted {
        statuses: [status::OK, status::UNSUPPORTED].into_iter().collect(),
    }));
    let options = ConnectOptions {
        client_name: "wp-pgcore".into(),
        app: "dspi-core tests".into(),
        app_version: "test".into(),
        token: None,
    };
    let (client, _) =
        LinkClient::connect(&TcpConnector::default(), &hub.url(), &options, None).unwrap();
    let serial = client.devices()[0].serial.clone();
    let inner = LinkTransport::new(client.clone(), 0, &serial);
    drop(inner.notifications());
    let (tx, rx) = mpsc::channel();
    let regen = Arc::new(AtomicBool::new(false));
    let mut w = Watched {
        inner,
        regen: Arc::clone(&regen),
        tx,
    };
    w.control_in_paged(cmds::SYS_PLATFORM, 0, 8).unwrap();
    assert!(!regen.load(Ordering::SeqCst));
    assert!(rx.try_recv().is_err());
    let r = w.control_in_paged(cmds::SYS_PLATFORM, 0, 8);
    assert_eq!(
        r.as_ref().err().and_then(LinkNotice::of),
        Some(LinkNotice::GenerationChanged)
    );
    assert!(regen.load(Ordering::SeqCst));
    assert!(matches!(rx.try_recv(), Ok(Msg::Kick)));
    // Nothing more is sent, and the supervisor is told once
    let sent = hub.requests().len();
    assert!(w.control_in_paged(cmds::SYS_PLATFORM, 0, 8).is_err());
    assert_eq!(hub.requests().len(), sent);
    assert!(rx.try_recv().is_err());
    client.close();
}
