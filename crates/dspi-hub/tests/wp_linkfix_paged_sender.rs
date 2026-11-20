//! WP LINKFIX: our hub and the core's in-process hub paths recognise the
//! dual-stack alink sender (FWPG4) on its paged stack, as PGSENDER's core does
//! over USB and DSPi Link (PLAN-PG section 1.9, `is_paged_alink_sender`).
//!
//! - The hub attaches and lists it, decides by the same rule on its own attach
//!   probe (with the USB product string), and asks it nothing of the DSP model:
//!   no image read (warm snapshots on), no poll of what it does not list.
//! - The app's in-process path (`LocalTransport`) and a principal's child
//!   (`PrincipalTransport`) pass the attachment's discovery on, so a core on top
//!   opens it as `AlinkSender` with the limited connection; before, the missing
//!   discovery made it fail closed. A client over DSPi Link sees the same.
//! - Fail closed: a paged DSP stays a DSP, even named as the sender with no
//!   outputs; its image is read and a core on top opens it as a DSP.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{DeviceKind, DspiCore, DspiError, ProtocolGeneration};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_link::messages::{LINK_13_MINOR, PollSpec};
use dspi_proto::generated::opcodes as op;
use dspi_proto::paged::{CmdId, EXT_PAGE, EXT_TUNNEL, PagedDir, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

const SENDER_PRODUCT: &str = "DSPi Audio Link Sender";
const WAIT: Duration = Duration::from_secs(10);

/// One request the device got on the wire: its page, command and direction.
type Seen = Arc<Mutex<Vec<(u8, u8, PagedDir)>>>;

/// A fake as the raw wire under the hub's attach probe, recording every request,
/// and for the fail-closed case naming the device as the sender with no outputs.
struct Rec {
    inner: FakeDevice,
    seen: Seen,
    descriptor: DeviceDescriptor,
    /// `SYS.PLATFORM` names no outputs.
    no_outputs: bool,
}

impl Rec {
    fn new(inner: FakeDevice) -> (Self, Seen) {
        let seen = Seen::default();
        let descriptor = inner.descriptor().clone();
        let rec = Self {
            inner,
            seen: Arc::clone(&seen),
            descriptor,
            no_outputs: false,
        };
        (rec, seen)
    }

    fn note(&self, page: u8, cmd: u8, dir: PagedDir) {
        self.seen.lock().unwrap().push((page, cmd, dir));
    }
}

impl Transport for Rec {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(0, opcode, PagedDir::Get);
        self.inner.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.note(0, opcode, PagedDir::Get);
        self.inner.control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.note(0, opcode, PagedDir::Set);
        self.inner.control_out(opcode, value, data)
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
        self.inner.carries_pages()
    }
    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.note(id.page(), id.cmd(), PagedDir::Get);
        let mut r = self.inner.control_in_paged(id, value, max_len)?;
        if self.no_outputs
            && id == cmds::SYS_PLATFORM
            && let Some(b) = r.get_mut(3)
        {
            *b = 0;
        }
        Ok(r)
    }
    fn control_out_paged(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        self.note(id.page(), id.cmd(), PagedDir::Set);
        self.inner.control_out_paged(id, value, data)
    }
}

/// An in-process hub that reads every attached device's image at once (as a hub
/// on the network does), so a sender asked for one would show it on the wire.
fn hub() -> Hub {
    Hub::start(HubConfig {
        warm_snapshots: true,
        ..HubConfig::in_process("test")
    })
    .unwrap()
}

/// A hub on its real server, open access on loopback.
fn hub_on_the_network() -> (Hub, String) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-linkfix").join(format!(
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

fn no_violation(remote: &FakeRemote) {
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}

/// What reached the sender on the wire: the probe's and the identity's SYS
/// reads and the tunnel, nothing else (no page 0, no bulk read, no DSP page).
fn only_sys_and_the_tunnel(seen: &Seen) {
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    for (page, cmd, dir) in &seen {
        assert!(
            *page == cmds::SYS_PLATFORM.page() || (*page == EXT_PAGE && *cmd == EXT_TUNNEL.cmd()),
            "0x{page:02X}{cmd:02X} {dir:?} reached the sender: {seen:?}"
        );
    }
}

/// A core on one of the hub's in-process transports, checked as PGSENDER checks
/// the sender over USB: the sender's kind, its own commands, nothing of the DSP.
fn check_sender_core(core: &DspiCore, seen: &Seen, path: &str) {
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, DeviceKind::AlinkSender, "{path}");
    assert_eq!(p.generation, ProtocolGeneration::Paged, "{path}");
    assert!(!p.pages.is_empty(), "{path}: the discovery's pages");
    core.fetch_all().unwrap();
    let s = core.get_state();
    assert!(!s.bulk_valid, "{path}");
    assert_eq!(s.num_output_channels, 0, "{path}");
    assert!(core.alink_info().unwrap().is_some(), "{path}");
    core.fetch_alink_status().unwrap();
    let before = seen.lock().unwrap().len();
    assert_eq!(
        core.set_master_volume(-13.0),
        Err(DspiError::Unsupported),
        "{path}"
    );
    assert_eq!(
        core.apply_all_params(),
        Err(DspiError::Unsupported),
        "{path}"
    );
    assert_eq!(seen.lock().unwrap().len(), before, "{path}: nothing sent");
}

#[test]
fn wp_linkfix_the_hub_attaches_the_paged_sender_and_asks_it_nothing_of_the_dsp() {
    let hub = hub();
    let dev = FakeDevice::sender(true).with_serial("LINKFIX-SNDR");
    let remote = dev.remote();
    let (rec, seen) = Rec::new(dev);
    let h = hub.attach(Box::new(rec), 0).unwrap();
    // Listed, with what the attach read: no outputs, paged, our extension channel.
    let listed = hub.devices();
    let d = listed
        .iter()
        .find(|d| d.identity.serial == "LINKFIX-SNDR")
        .unwrap();
    assert_eq!(d.identity.outputs, 0);
    assert!(d.identity.protocol.is_some());
    assert_eq!(d.identity.ext, Some(true));
    // No image: `device.snapshot` answers at once, with nothing read.
    let start = Instant::now();
    assert!(hub.handle().snapshot(h).is_err());
    assert!(start.elapsed() < WAIT / 2, "{:?}", start.elapsed());
    // Polls: a DSP read (by page, or page 0 from a 1.2 client) is refused; a
    // read the sender lists is granted.
    let s = hub.open_session(Role::Viewer, None).unwrap();
    s.set_link_minor(LINK_13_MINOR);
    let poll = |slot: u8, id: CmdId| PollSpec {
        slot,
        req: id.cmd(),
        val: 0,
        idx: id.w_index(),
        len: 4,
        hz: 10,
    };
    let refused = hub
        .poll_subscribe(&s, h, &[poll(0, cmds::ROUTE_MASTER_VOLUME)])
        .unwrap_err();
    assert_eq!(refused.code(), "unsupported");
    let old = hub.open_session(Role::Viewer, None).unwrap();
    old.set_link_minor(2);
    let meters = PollSpec {
        req: op::REQ_GET_STATUS,
        idx: dspi_proto::VENDOR_INTERFACE,
        ..poll(1, cmds::SYS_PLATFORM)
    };
    assert_eq!(
        hub.poll_subscribe(&old, h, &[meters]).unwrap_err().code(),
        "unsupported"
    );
    assert!(
        hub.poll_subscribe(&s, h, &[poll(2, cmds::SYS_PLATFORM)])
            .is_ok()
    );
    // Give the worker time for anything it would have read on its own.
    std::thread::sleep(Duration::from_millis(300));
    only_sys_and_the_tunnel(&seen);
    no_violation(&remote);
}

#[test]
fn wp_linkfix_a_core_on_the_in_process_paths_opens_the_paged_sender() {
    let hub = hub();
    let h = hub.handle();
    let serial = "LINKFIX-SNDL";
    let dev = FakeDevice::sender(true).with_serial(serial);
    let remote = dev.remote();
    let (rec, seen) = Rec::new(dev);
    hub.attach(Box::new(rec), 0).unwrap();

    // The app's in-process path.
    let local = h.open_local(serial).unwrap();
    assert!(local.paged_discovery().is_some(), "the discovery passed on");
    let core = DspiCore::new();
    core.open_transport(Box::new(local), serial.into(), 0)
        .unwrap();
    check_sender_core(&core, &seen, "local");
    core.disconnect();

    // A principal's child.
    let parent = h.open_session(Role::Admin, None).unwrap();
    let child = h.open_child(&parent).unwrap();
    let principal = h.open_as(serial, &child, true).unwrap();
    assert_eq!(
        principal.paged_discovery(),
        h.open_local(serial).unwrap().paged_discovery()
    );
    let core = DspiCore::new();
    core.open_transport(Box::new(principal), serial.into(), 0)
        .unwrap();
    check_sender_core(&core, &seen, "principal");
    core.disconnect();

    only_sys_and_the_tunnel(&seen);
    no_violation(&remote);
}

#[test]
fn wp_linkfix_a_client_over_dspi_link_gets_the_paged_sender_from_our_hub() {
    let (hub, url) = hub_on_the_network();
    let serial = "LINKFIX-SNDN";
    let dev = FakeDevice::sender(true).with_serial(serial);
    let remote = dev.remote();
    let (rec, seen) = Rec::new(dev);
    hub.attach(Box::new(rec), 0).unwrap();
    let core = DspiCore::new();
    core.link_connect(
        url,
        "wp-linkfix".into(),
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
    check_sender_core(&core, &seen, "link");
    core.link_close();
    only_sys_and_the_tunnel(&seen);
    no_violation(&remote);
}

#[test]
fn wp_linkfix_a_paged_dsp_stays_a_dsp_behind_the_hub() {
    for (i, g) in [FakeGeneration::Bofr117, FakeGeneration::STOCK117]
        .into_iter()
        .enumerate()
    {
        let hub = hub();
        let h = hub.handle();
        let serial = format!("LINKFIX-DSP{i}");
        let dev = FakeDevice::demo_generation(g).with_serial(&serial);
        let remote = dev.remote();
        // Named as the sender and with no outputs: its pages still say DSP.
        let (mut rec, seen) = Rec::new(dev);
        rec.descriptor.product = Some(SENDER_PRODUCT.into());
        rec.no_outputs = true;
        let handle = hub.attach(Box::new(rec), 0).unwrap();
        // The hub reads its image as any DSP's.
        let snap = hub.handle().snapshot(handle);
        assert!(snap.is_ok(), "{g:?}: {snap:?}");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(p, c, _)| CmdId::new(*p, *c) == cmds::STORE_ALL_PARAMS
                    || CmdId::new(*p, *c) == cmds::STORE_ALL_PARAMS_CHUNK),
            "{g:?}: the image read"
        );
        // A DSP poll is granted.
        let s = hub.open_session(Role::Viewer, None).unwrap();
        s.set_link_minor(LINK_13_MINOR);
        let volume = PollSpec {
            slot: 0,
            req: cmds::ROUTE_MASTER_VOLUME.cmd(),
            val: 0,
            idx: cmds::ROUTE_MASTER_VOLUME.w_index(),
            len: 4,
            hz: 10,
        };
        assert!(hub.poll_subscribe(&s, handle, &[volume]).is_ok(), "{g:?}");
        // A core on the in-process path: a DSP, with its image.
        let local = h.open_local(&serial).unwrap();
        assert!(local.paged_discovery().is_some(), "{g:?}");
        let core = DspiCore::new();
        core.open_transport(Box::new(local), serial.clone(), 0)
            .unwrap_or_else(|e| panic!("{g:?}: {e}"));
        let p = core.device_protocol().unwrap();
        assert_eq!(p.device_kind, DeviceKind::Dsp, "{g:?}");
        assert!(core.get_state().bulk_valid, "{g:?}");
        core.set_master_volume(-9.0)
            .unwrap_or_else(|e| panic!("{g:?}: {e}"));
        core.disconnect();
        no_violation(&remote);
    }
}
