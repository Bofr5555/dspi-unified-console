//! WP PGFIX: the DSPi Link bind behind a 1.2 hub (PGMAT F2) and the alink
//! sender over DSPi Link (PGMAT F1). Until `REQ_GET_PLATFORM` has answered,
//! nothing else reaches the device: no page-0 meter poll goes to a paged
//! device (it ends in `link_needs_13_hub`), and a legacy one gets its poll
//! after the platform. The sender binds with its own commands only and gets
//! no meter poll at all.

use dspi_link::fakehub::{Auth, DeviceRequest, FakeHub, HubConfig, HubDevice, SharedDevice};
use dspi_link::status;
use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::{Capture, FakeGeneration};
use dspi_transport::link::LinkNotice;
use dspi_transport::{FakeDevice, FakeOptions, FakeRemote};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A fake as a 1.2 hub forwards to it: page 0 only (the core sends no page
/// through a 1.2 hub).
struct OnHub(FakeDevice);

impl HubDevice for OnHub {
    fn request(
        &mut self,
        dir: Dir,
        b_request: u8,
        w_value: u16,
        _w_index: u16,
        w_length: u16,
        payload: &[u8],
    ) -> (u8, Vec<u8>) {
        let r = match dir {
            Dir::Get => self.0.control_in_upto(b_request, w_value, w_length),
            Dir::Set => self
                .0
                .control_out(b_request, w_value, payload)
                .map(|()| Vec::new()),
        };
        match r {
            Ok(d) => (status::OK, d),
            Err(TransportError::Disconnected) => (status::NO_DEVICE, Vec::new()),
            Err(_) => (status::ERROR, Vec::new()),
        }
    }
}

/// A 1.2 hub forwarding the page, open access, in front of `dev`.
fn l12(dev: FakeDevice) -> (FakeHub, FakeRemote) {
    let remote = dev.remote();
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::l12_forward(Some(SharedDevice::new(OnHub(dev))))
    })
    .unwrap();
    (hub, remote)
}

fn bind(hub: &FakeHub) -> (Arc<DspiCore>, Result<()>) {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-pgfix".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    let selected = core.link_select_device(d.handle, d.serial);
    (core, selected)
}

fn is_meter_poll(r: &DeviceRequest) -> bool {
    r.b_request == op::REQ_GET_STATUS && r.w_value == 9
}

#[test]
fn wp_pgfix_a_paged_device_behind_a_12_hub_gets_the_platform_read_only() {
    let dev = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            allow_violation: true,
            ..FakeGeneration::STOCK117.options()
        },
    );
    let (hub, remote) = l12(dev);
    let (core, selected) = bind(&hub);
    let e = selected.unwrap_err();
    assert!(
        e.to_string().contains(LinkNotice::Needs13Hub.text()),
        "{e:?}"
    );
    // Long enough for a 10 Hz poll to show.
    std::thread::sleep(Duration::from_millis(400));
    let sent = hub.requests();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].b_request, op::REQ_GET_PLATFORM, "{sent:?}");
    // The one page-0 read the paged fake refused (let through and counted).
    assert_eq!(remote.violations(), 1, "{:?}", remote.last_violation());
    core.link_close();
}

#[test]
fn wp_pgfix_a_legacy_device_gets_its_meter_poll_after_the_platform() {
    let dev = FakeDevice::new(Capture::demo(), FakeGeneration::Stock116.options());
    let (hub, remote) = l12(dev);
    let (core, selected) = bind(&hub);
    selected.unwrap();
    let _ = core.fetch_status();
    wait_until("the meter poll", || {
        hub.requests().iter().any(is_meter_poll)
    });
    let sent = hub.requests();
    assert_eq!(sent[0].b_request, op::REQ_GET_PLATFORM, "{sent:?}");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    core.link_close();
}

#[test]
fn wp_pgfix_the_sender_binds_over_link_without_meters() {
    let (hub, remote) = l12(FakeDevice::sender(false));
    let (core, selected) = bind(&hub);
    selected.unwrap();
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, crate::DeviceKind::AlinkSender);
    core.fetch_all().unwrap();
    assert!(!core.get_state().bulk_valid);
    // Open access: admin, so the extension channel and its alink block.
    assert!(!core.ext_status().unwrap().supported.is_empty());
    assert!(core.alink_info().unwrap().is_some());
    assert!(core.meter_path().is_none(), "no meter path for the sender");
    assert_eq!(core.set_master_volume(-13.0), Err(DspiError::Unsupported));
    assert_eq!(core.fetch_status().map(drop), Err(DspiError::Unsupported));
    std::thread::sleep(Duration::from_millis(400));
    let sent = hub.requests();
    assert!(!sent.iter().any(is_meter_poll), "{sent:?}");
    assert_eq!(sent[0].b_request, op::REQ_GET_PLATFORM, "{sent:?}");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    core.link_close();
}
