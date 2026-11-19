//! WP PGSENDER: the dual-stack alink sender (FWPG4) over DSPi Link. Behind a
//! 1.3 hub it is paged and binds as `DeviceKind::AlinkSender` with no meter
//! poll and nothing but its SYS reads and the tunnel on the wire; behind a
//! 1.2 hub the core speaks legacy only and it binds as the legacy sender (its
//! page-0 stack).

use dspi_link::fakehub::{Auth, DeviceRequest, FakeHub, HubConfig, HubDevice, SharedDevice};
use dspi_link::status;
use dspi_proto::paged::{EXT_PAGE, EXT_TUNNEL, LastResult, PagedDir, cmds};
use dspi_transport::{CmdId, FakeDevice, FakeRemote};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The fake as a USB hub forwards to it: page 0 as the 1.1.6 calls, a page
/// as the paged calls; a paged STALL explained by `SYS.LAST_RESULT` when
/// `explain` (a 1.3 hub). As WP PGCORE's.
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

fn hub(cfg: fn(Option<SharedDevice>) -> HubConfig, explain: bool) -> (FakeHub, FakeRemote) {
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let device = SharedDevice::new(OnHub { dev, explain });
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..cfg(Some(device))
    })
    .unwrap();
    (hub, remote)
}

fn bind(hub: &FakeHub) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-pgsender".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core
}

/// The sender's commands as the core may send them: its SYS reads and the
/// tunnel (a GET, or a SET of the alink block).
fn senders_own(r: &DeviceRequest) -> bool {
    let page = r.sent_page();
    page == cmds::SYS_PLATFORM.page() || (page == EXT_PAGE && r.b_request == EXT_TUNNEL.cmd())
}

#[test]
fn wp_pgsender_the_paged_sender_binds_behind_a_13_hub_without_meters() {
    let (hub, remote) = hub(HubConfig::l13, true);
    let core = bind(&hub);
    core.ensure_probed();
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, crate::DeviceKind::AlinkSender);
    assert_eq!(p.generation, crate::ProtocolGeneration::Paged);
    assert_eq!(p.ext_envelope, crate::ExtEnvelope::Tunnel);
    core.fetch_all().unwrap();
    assert!(!core.get_state().bulk_valid);
    // Open access: the extension channel and its alink block, by the tunnel.
    assert!(!core.ext_status().unwrap().supported.is_empty());
    assert!(core.alink_info().unwrap().is_some());
    assert!(core.meter_path().is_none(), "no meter path for the sender");
    // No meter block was ever wanted from the sender (none asked at the bind
    // from the hub's listed channel count).
    let side = {
        let inner = core.lock();
        inner
            .link
            .as_ref()
            .and_then(|s| s.bound.as_ref())
            .and_then(|b| b.side.clone())
            .expect("the bound device's side channel")
    };
    assert_eq!(guard(&side.feed).meter_len, 0, "a meter poll was wanted");
    assert_eq!(core.set_master_volume(-13.0), Err(DspiError::Unsupported));
    assert_eq!(core.fetch_status().map(drop), Err(DspiError::Unsupported));
    // Long enough for a 10 Hz poll to show.
    std::thread::sleep(Duration::from_millis(400));
    let sent = hub.requests();
    let other: Vec<_> = sent.iter().filter(|r| !senders_own(r)).collect();
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    core.link_close();
}

#[test]
fn wp_pgsender_behind_a_12_hub_the_dual_stack_sender_is_the_legacy_sender() {
    let (hub, remote) = hub(HubConfig::l12_forward, false);
    let core = bind(&hub);
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, crate::DeviceKind::AlinkSender);
    assert_eq!(p.generation, crate::ProtocolGeneration::Legacy);
    assert!(core.alink_info().unwrap().is_some());
    assert!(core.meter_path().is_none(), "no meter path for the sender");
    std::thread::sleep(Duration::from_millis(400));
    let sent = hub.requests();
    assert!(sent.iter().all(|r| r.sent_page() == 0), "{sent:?}");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    core.link_close();
}
