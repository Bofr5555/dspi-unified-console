//! WP PGSENDER: the dual-stack alink sender (FWPG4) on its paged stack opens
//! as `DeviceKind::AlinkSender`, as the guarded sender does on the legacy
//! one, with its limited connection (device info, the audio link through the
//! tunnel, the update check). Fail closed: a paged device that does not meet
//! every condition of [`is_paged_alink_sender`] is not a sender, a paged DSP
//! stays a DSP, and nothing DSP-only reaches the sender.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::{Arc, Mutex};

use dspi_proto::paged::{EXT_PAGE, EXT_TUNNEL, PageCommands, PageSet, PagedDir, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::mock::Reply;
use dspi_transport::paged::{self, Discovery};
use dspi_transport::{
    CmdId, DeviceDescriptor, Generation, MockTransport, NotificationSource, TransportCaps,
};

use super::*;
use crate::{DeviceKind, ExtEnvelope, ProtocolGeneration};

const SENDER_PRODUCT: &str = "DSPi Audio Link Sender";

/// One request the device got: its page, command and direction.
type Seen = Arc<Mutex<Vec<(u8, u8, PagedDir)>>>;

/// A fake as the raw wire, recording every request, and (for the fail-closed
/// cases) changing what the device says about itself.
struct Rec {
    inner: FakeDevice,
    seen: Seen,
    descriptor: DeviceDescriptor,
    /// `SYS.PLATFORM` names no outputs.
    no_outputs: bool,
    /// The bulk image's GETs STALL on the wire (a bulk read that fails).
    fail_bulk: bool,
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
            fail_bulk: false,
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
        if self.fail_bulk && is_bulk(id) {
            return Err(TransportError::Stalled { opcode: id.cmd() });
        }
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

/// `STORE.ALL_PARAMS` and its chunk form: the bulk image.
fn is_bulk(id: CmdId) -> bool {
    id == cmds::STORE_ALL_PARAMS || id == cmds::STORE_ALL_PARAMS_CHUNK
}

fn open(rec: Rec) -> (Arc<DspiCore>, Result<()>) {
    let serial = rec.descriptor.serial.clone();
    let core = DspiCore::new();
    let t = paged::open(Box::new(rec)).unwrap();
    assert!(matches!(t.generation(), Some(Generation::Paged(_))));
    let r = core.open_transport(t, serial, 0);
    (core, r)
}

/// The paged discovery `paged::open` reads from `dev`.
fn discovery_of(dev: FakeDevice) -> Discovery {
    paged::open(Box::new(dev))
        .unwrap()
        .paged_discovery()
        .expect("a paged device")
}

fn bitmap(cmds: &[(u8, PagedDir)]) -> PageCommands {
    let mut m = PageCommands::EMPTY;
    for (c, d) in cmds {
        m.insert(*c, *d);
    }
    m
}

/// The sender's page set as FWPG4 is to list it (the fake's), built by hand,
/// with `extra` pages and commands on top.
fn sender_pages(tunnel: bool, extra: &[(CmdId, PagedDir)]) -> PageSet {
    let (sys, store) = (cmds::SYS_PLATFORM.page(), cmds::STORE_ALL_PARAMS.page());
    let mut by_page: std::collections::BTreeMap<u8, Vec<(u8, PagedDir)>> = Default::default();
    for id in [
        cmds::SYS_PROTOCOL_INFO,
        cmds::SYS_PAGE_LIST,
        cmds::SYS_PAGE_COMMANDS,
        cmds::SYS_PLATFORM,
        cmds::SYS_SERIAL,
    ] {
        by_page
            .entry(sys)
            .or_default()
            .push((id.cmd(), PagedDir::Get));
    }
    by_page
        .entry(store)
        .or_default()
        .push((cmds::STORE_OUTPUT_CONFIG_SAVE.cmd(), PagedDir::Set));
    if tunnel {
        let e = by_page.entry(EXT_PAGE).or_default();
        e.push((EXT_TUNNEL.cmd(), PagedDir::Get));
        e.push((EXT_TUNNEL.cmd(), PagedDir::Set));
    }
    for (id, dir) in extra {
        by_page.entry(id.page()).or_default().push((id.cmd(), *dir));
    }
    let mut s = PageSet::new();
    for (page, list) in by_page {
        s.insert(page, 1, bitmap(&list));
    }
    s
}

#[test]
fn wp_pgsender_the_rule_on_the_discovery() {
    let sender = discovery_of(FakeDevice::sender(true));
    // The fake of FWPG4's sender meets every condition, with or without the
    // product string (MIDI, behind a hub: the platform names no outputs).
    assert_eq!(sender.platform.get(3), Some(&0), "{:?}", sender.platform);
    assert!(is_paged_alink_sender(&sender, Some(SENDER_PRODUCT)));
    assert!(is_paged_alink_sender(&sender, None));
    // The hand-built set is the fake's.
    let mut d = sender.clone();
    d.pages = sender_pages(true, &[]);
    assert!(is_paged_alink_sender(&d, None));

    // 1. Identity: another product string, or outputs without a product.
    assert!(!is_paged_alink_sender(&sender, Some("Weeb Labs DSPi")));
    let mut d = sender.clone();
    d.platform[3] = 4;
    assert!(!is_paged_alink_sender(&d, None));
    assert!(is_paged_alink_sender(&d, Some(SENDER_PRODUCT)));
    d.platform.truncate(3);
    assert!(
        !is_paged_alink_sender(&d, Some(SENDER_PRODUCT)),
        "no platform"
    );
    // 2. Not our firmware: no tunnel (the page without it, or no page 0xE0).
    let mut d = sender.clone();
    d.pages = sender_pages(false, &[]);
    assert!(!is_paged_alink_sender(&d, Some(SENDER_PRODUCT)));
    d.pages.insert(EXT_PAGE, 1, PageCommands::EMPTY);
    assert!(!is_paged_alink_sender(&d, Some(SENDER_PRODUCT)));
    // 3. A bulk image.
    let mut d = sender.clone();
    d.pages = sender_pages(true, &[(cmds::STORE_ALL_PARAMS, PagedDir::Get)]);
    assert!(!is_paged_alink_sender(&d, Some(SENDER_PRODUCT)));
    // 4. Any other page (a DSP page), even with nothing on it.
    let mut d = sender.clone();
    d.pages = sender_pages(true, &[]);
    d.pages
        .insert(cmds::ROUTE_MASTER_VOLUME.page(), 1, PageCommands::EMPTY);
    assert!(!is_paged_alink_sender(&d, Some(SENDER_PRODUCT)));

    // Paged DSPs, ours and stock, are not senders, whatever they are called.
    for g in [FakeGeneration::Bofr117, FakeGeneration::STOCK117] {
        let mut dsp = discovery_of(FakeDevice::demo_generation(g));
        assert!(!is_paged_alink_sender(&dsp, Some(SENDER_PRODUCT)), "{g:?}");
        dsp.platform[3] = 0;
        assert!(!is_paged_alink_sender(&dsp, None), "{g:?}");
    }
}

#[test]
fn wp_pgsender_the_paged_sender_connects_with_its_own_commands() {
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let (rec, seen) = Rec::new(dev);
    let (core, r) = open(rec);
    r.unwrap();
    let p = core.device_protocol().unwrap();
    assert_eq!(p.device_kind, DeviceKind::AlinkSender);
    assert_eq!(p.generation, ProtocolGeneration::Paged);
    assert_eq!(p.ext_envelope, ExtEnvelope::Tunnel);
    // The device info: the platform's identity, no image, no channels.
    core.fetch_all().unwrap();
    let s = core.get_state();
    assert!(!s.bulk_valid);
    assert_eq!(
        (s.num_channels, s.num_input_channels, s.num_output_channels),
        (0, 0, 0)
    );
    assert_ne!(s.fw_major, 0, "the firmware version is read");
    // The extension channel (through the tunnel) and its alink block.
    assert!(!core.ext_status().unwrap().supported.is_empty());
    assert!(core.alink_info().unwrap().is_some());
    core.fetch_alink_status().unwrap();
    core.reset_alink_stats().unwrap();
    // The update check's target: a Pico with BOOTSEL.
    let target = core.firmware_target();
    assert!(target.has_bootsel(), "{target:?}");
    // Nothing of the DSP model: refused before anything is sent.
    let before = seen.lock().unwrap().len();
    assert_eq!(core.set_master_volume(-13.0), Err(DspiError::Unsupported));
    assert_eq!(core.apply_all_params(), Err(DspiError::Unsupported));
    assert_eq!(core.refresh_params(), Err(DspiError::Unsupported));
    assert_eq!(core.fetch_status().map(drop), Err(DspiError::Unsupported));
    assert_eq!(seen.lock().unwrap().len(), before, "nothing sent");
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    // On the wire: the probe, the sender's SYS reads and the tunnel only:
    // no page 0 (it spoke paged), no bulk read, no DSP page.
    let seen = seen.lock().unwrap().clone();
    for (page, cmd, dir) in &seen {
        assert!(
            *page == cmds::SYS_PLATFORM.page() || (*page == EXT_PAGE && *cmd == EXT_TUNNEL.cmd()),
            "0x{page:02X}{cmd:02X} {dir:?} reached the sender: {seen:?}"
        );
    }
    assert_eq!(
        core.device_protocol().unwrap().device_kind,
        DeviceKind::AlinkSender
    );
}

#[test]
fn wp_pgsender_a_paged_dsp_stays_a_dsp() {
    // A paged DSP named as the sender, with no outputs: its bulk image reads,
    // so it is a DSP.
    let dev = FakeDevice::demo_generation(FakeGeneration::Bofr117);
    let remote = dev.remote();
    let (mut rec, _) = Rec::new(dev);
    rec.descriptor.product = Some(SENDER_PRODUCT.into());
    rec.no_outputs = true;
    let (core, r) = open(rec);
    r.unwrap();
    assert_eq!(core.device_protocol().unwrap().device_kind, DeviceKind::Dsp);
    assert!(core.get_state().bulk_valid);
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());

    // The same DSP whose bulk read fails on the wire: it lists the bulk
    // image, so it is not a sender; the open fails as before.
    for g in [FakeGeneration::Bofr117, FakeGeneration::STOCK117] {
        let dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        let (mut rec, seen) = Rec::new(dev);
        rec.descriptor.product = Some(SENDER_PRODUCT.into());
        rec.no_outputs = true;
        rec.fail_bulk = true;
        let (core, r) = open(rec);
        assert!(r.is_err(), "{g:?}: {r:?}");
        assert!(core.device_protocol().is_err(), "{g:?}: nothing connected");
        assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
        // Nothing past the failed bulk read: no tunnel read for the sender.
        let seen = seen.lock().unwrap().clone();
        let bulk = seen
            .iter()
            .rposition(|(p, c, _)| is_bulk(CmdId::new(*p, *c)))
            .expect("the bulk read");
        let after: Vec<_> = seen[bulk + 1..]
            .iter()
            .filter(|(p, c, _)| CmdId::new(*p, *c) != cmds::SYS_LAST_RESULT)
            .collect();
        assert!(after.is_empty(), "{g:?}: {after:?}");
    }
}

/// `SYS.PROTOCOL_INFO`: "DSPi", major 1, minor 0, `pages`, flags, 64-byte
/// SET and GET limits.
fn protocol_info(pages: u8) -> Vec<u8> {
    let mut r = b"DSPi".to_vec();
    r.extend([1, 0, pages, 0, 64, 0, 64, 0]);
    r.resize(16, 0);
    r
}

#[test]
fn wp_pgsender_an_unrecognised_paged_device_is_not_a_sender() {
    // Called the sender, no outputs, only SYS: no tunnel, so not our sender.
    // The open fails as for any paged device without a bulk image, and
    // nothing but SYS reads went out.
    let sys = bitmap(&[
        (cmds::SYS_PROTOCOL_INFO.cmd(), PagedDir::Get),
        (cmds::SYS_PAGE_LIST.cmd(), PagedDir::Get),
        (cmds::SYS_PAGE_COMMANDS.cmd(), PagedDir::Get),
        (cmds::SYS_PLATFORM.cmd(), PagedDir::Get),
        (cmds::SYS_SERIAL.cmd(), PagedDir::Get),
    ]);
    let mock = MockTransport::new()
        .carrying_pages()
        .with_descriptor(DeviceDescriptor {
            serial: "PGSNDR01".into(),
            product: Some(SENDER_PRODUCT.into()),
            ..DeviceDescriptor::default()
        })
        .paged(cmds::SYS_PROTOCOL_INFO, Reply::Data(protocol_info(1)))
        .paged(
            cmds::SYS_PAGE_LIST,
            Reply::Data(vec![cmds::SYS_PLATFORM.page(), 1]),
        )
        .paged(cmds::SYS_PAGE_COMMANDS, Reply::Data(sys.encode().to_vec()))
        .paged(
            cmds::SYS_PLATFORM,
            Reply::Data(vec![2, 1, 0x17, 0, 1, 7, 1]),
        )
        .paged(cmds::SYS_SERIAL, Reply::Data(b"PGSNDR01".to_vec()));
    let log = mock.log_handle();
    let core = DspiCore::new();
    let t = paged::open(Box::new(mock)).unwrap();
    let disc = t.paged_discovery().unwrap();
    assert!(!is_paged_alink_sender(&disc, Some(SENDER_PRODUCT)));
    let r = core.open_transport(t, "PGSNDR01".into(), 0);
    assert!(r.is_err(), "{r:?}");
    assert!(core.device_protocol().is_err(), "nothing connected");
    let log = log.lock().unwrap().clone();
    assert!(
        log.iter().all(|e| e.page() == cmds::SYS_PLATFORM.page()),
        "{log:?}"
    );
}

#[test]
fn wp_pgsender_a_paged_connection_without_discovery_is_not_a_sender() {
    // A wrapper that says "paged" but passes no discovery on (the hub's
    // in-process path): the rule cannot be checked, so no sender.
    struct NoDisc(Box<dyn Transport>);
    impl Transport for NoDisc {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.0.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.0.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.0.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn generation(&self) -> Option<Generation> {
            self.0.generation()
        }
    }
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let t = NoDisc(paged::open(Box::new(dev)).unwrap());
    assert!(t.paged_discovery().is_none());
    let core = DspiCore::new();
    assert!(core.open_transport(Box::new(t), serial, 0).is_err());
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}
