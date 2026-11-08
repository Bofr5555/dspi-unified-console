//! WP CG33: an edit made after a preset load, before the load's notification
//! has been processed, stays unsaved. The device below holds its
//! notifications back until the test lets them through, so the window
//! between the load and its `BULK_INVALIDATED` (up to ~100 ms on USB, longer
//! over DSPi Link) is as wide as each case needs, deterministically: the
//! load by this client, by another host (a second client, a Link session)
//! and by the device's own controls (a knob, IR), over USB-like attribution
//! (no origin) and Link-like attribution (origin per packet, 0 for the
//! device's own changes).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_proto::generated::{self as g, opcodes as op};
use dspi_transport::{
    Attributed, DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport,
    TransportCaps, TransportError,
};

use super::*;

const WAIT: Duration = Duration::from_secs(8);
/// This client's session on the Link-like device.
const OWN_SESSION: u16 = 1;
/// Another Link client's session.
const OTHER_SESSION: u16 = 9;
const SRC_HOST: u8 = nfy::PARAM_SRC_HOST_SET as u8;
const SRC_UART: u8 = nfy::PARAM_SRC_UART as u8;

/// The packets the device has sent, held until the gate opens.
#[derive(Default)]
struct Gate {
    open: AtomicBool,
    held: Mutex<VecDeque<Attributed>>,
    /// Read calls started, and packets handed to the reader.
    reads: AtomicU64,
    handed: AtomicU64,
    /// The image reads time out while set (a re-read that fails).
    failing: AtomicBool,
}

/// The demo device, shared with the test (to act as another host), with
/// its notifications behind a [`Gate`]. `link`: Link-like caps, session and
/// origins (as the hub stamps them: the writer's session for a write on its
/// own path, 0 for anything the device did itself).
struct Gated {
    dev: Arc<Mutex<FakeDevice>>,
    descriptor: DeviceDescriptor,
    gate: Arc<Gate>,
    link: bool,
}

impl Gated {
    fn dev(&self) -> std::sync::MutexGuard<'_, FakeDevice> {
        self.dev.lock().unwrap()
    }
}

impl Gated {
    fn image_read_fails(&self, opcode: u8) -> dspi_transport::Result<()> {
        if matches!(
            opcode,
            op::REQ_GET_ALL_PARAMS | op::REQ_GET_ALL_PARAMS_CHUNK
        ) && self.gate.failing.load(Ordering::SeqCst)
        {
            return Err(TransportError::TimedOut { opcode, retries: 0 });
        }
        Ok(())
    }
}

impl Transport for Gated {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.image_read_fails(opcode)?;
        self.dev().control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.image_read_fails(opcode)?;
        self.dev().control_in_upto(opcode, value, len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.dev().control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.dev().max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let inner = self.dev().notifications()?;
        Some(Box::new(GatedSource {
            inner,
            gate: Arc::clone(&self.gate),
            link: self.link,
        }))
    }
    fn caps(&self) -> TransportCaps {
        if self.link {
            TransportCaps {
                kind: TransportKind::Link,
                chunked_bulk: false,
                remote: false,
            }
        } else {
            self.dev().caps()
        }
    }
    fn session_id(&self) -> Option<u16> {
        self.link.then_some(OWN_SESSION)
    }
}

struct GatedSource {
    inner: Box<dyn NotificationSource>,
    gate: Arc<Gate>,
    link: bool,
}

impl GatedSource {
    /// The hub's attribution: `HOST_SET` is this client's write, `UART`
    /// stands for another Link client's, the rest is the device's own.
    fn origin(&self, packet: &[u8]) -> Option<u16> {
        if !self.link {
            return None;
        }
        let source = match dspi_session::notify::decode(packet) {
            Ok((_, Event::ParamChanged { source, .. } | Event::BulkInvalidated { source })) => {
                source.raw()
            }
            _ => 0,
        };
        Some(match source {
            SRC_HOST => OWN_SESSION,
            SRC_UART => OTHER_SESSION,
            _ => 0,
        })
    }
}

impl NotificationSource for GatedSource {
    fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
        Ok(self.read_attributed(timeout)?.packet)
    }

    fn read_attributed(&mut self, _timeout: Duration) -> dspi_transport::Result<Attributed> {
        self.gate.reads.fetch_add(1, Ordering::SeqCst);
        loop {
            let packet = self.inner.read(Duration::ZERO)?;
            if packet.is_empty() {
                break;
            }
            let origin = self.origin(&packet);
            self.gate
                .held
                .lock()
                .unwrap()
                .push_back(Attributed { packet, origin });
        }
        if self.gate.open.load(Ordering::SeqCst)
            && let Some(a) = self.gate.held.lock().unwrap().pop_front()
        {
            self.gate.handed.fetch_add(1, Ordering::SeqCst);
            return Ok(a);
        }
        std::thread::sleep(Duration::from_millis(1));
        Ok(Attributed {
            packet: Vec::new(),
            origin: None,
        })
    }
}

struct Rig {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    /// The device, to act as another host or the device's own controls.
    dev: Arc<Mutex<FakeDevice>>,
    gate: Arc<Gate>,
}

/// Preset 0 (active) and preset 1 differ in input 0's preamp (-2 / -4 dB);
/// everything delivered, the baseline preset 0's.
fn rig(link: bool) -> Rig {
    let fake = FakeDevice::demo();
    let remote = fake.remote();
    let descriptor = fake.descriptor().clone();
    let dev = Arc::new(Mutex::new(fake));
    let gate = Arc::new(Gate::default());
    gate.open.store(true, Ordering::SeqCst);
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(Gated {
            dev: Arc::clone(&dev),
            descriptor,
            gate: Arc::clone(&gate),
            link,
        }),
        "DEMO".into(),
        0,
    )
    .unwrap();
    let rig = Rig {
        core,
        remote,
        dev,
        gate,
    };
    rig.deliver();
    rig.core.set_input_preamp(0, -4.0).unwrap();
    rig.core.save_preset(1).unwrap();
    rig.core.set_input_preamp(0, -2.0).unwrap();
    rig.core.save_preset(0).unwrap();
    rig.deliver();
    assert!(!rig.core.preset_dirty(), "{:?}", rig.core.preset_changes());
    rig
}

impl Rig {
    fn hold(&self) {
        self.gate.open.store(false, Ordering::SeqCst);
    }

    /// Let every held packet through, wait until the reader has queued them
    /// all, and process them in one batch.
    fn deliver(&self) -> NotifyResult {
        self.gate.open.store(true, Ordering::SeqCst);
        let start = Instant::now();
        loop {
            assert!(
                start.elapsed() < WAIT,
                "the reader did not take the packets"
            );
            let handed = self.gate.handed.load(Ordering::SeqCst);
            let reads = self.gate.reads.load(Ordering::SeqCst);
            if !self.gate.held.lock().unwrap().is_empty() {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            // Two more read calls with nothing handed out: the last packet
            // handed out before them is in the reader's queue.
            while self.gate.reads.load(Ordering::SeqCst) < reads + 2 {
                assert!(start.elapsed() < WAIT, "the reader stopped reading");
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.gate.handed.load(Ordering::SeqCst) == handed
                && self.gate.held.lock().unwrap().is_empty()
            {
                break;
            }
        }
        self.core.process_notifications()
    }

    /// Another host, or the device's own controls, loads `slot` (the
    /// firmware's path: `PRESET_LOADED`, then `BULK_INVALIDATED` source
    /// `PRESET`).
    fn load_elsewhere(&self, slot: u8) {
        let reply = self
            .dev
            .lock()
            .unwrap()
            .control_in(op::REQ_PRESET_LOAD, u16::from(slot), 1)
            .unwrap();
        assert_eq!(reply, [0], "the fake refused the load");
    }

    /// The load's notification arrived: the mirror is the device's.
    fn delivered_load(&self) {
        let r = self.deliver();
        assert_ne!(r.flags & NOTIFY_REFRESHED, 0, "no re-read: {r:?}");
        assert_eq!(self.core.get_state().bulk_raw, self.remote.image());
    }

    fn changes(&self) -> Vec<String> {
        let changes = self.core.preset_changes();
        assert_eq!(self.core.preset_dirty(), !changes.is_empty());
        changes
    }
}

fn preamp(core: &DspiCore, input: usize) -> f32 {
    core.get_state().input_preamp_db[input]
}

/// This client loads, edits before the load's notification arrives: the
/// edit is unsaved against the loaded preset's own value, and nothing else
/// is.
fn own_load_then_edit(link: bool) {
    let rig = rig(link);
    rig.hold();
    rig.core.load_preset(1).unwrap();
    assert_eq!(preamp(&rig.core, 0), -4.0);
    assert!(!rig.core.preset_dirty());
    rig.core.set_input_preamp(0, 1.0).unwrap();
    rig.core.set_input_preamp(1, -1.5).unwrap();
    rig.delivered_load();
    let changes = rig.changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes[0].contains("-4.0 dB → 1.0 dB"), "{changes:?}");
    assert!(changes[1].contains("0.0 dB → -1.5 dB"), "{changes:?}");
}

#[test]
fn wp_cg33_an_edit_after_own_load_stays_unsaved_usb() {
    own_load_then_edit(false);
}

#[test]
fn wp_cg33_an_edit_after_own_load_stays_unsaved_link() {
    own_load_then_edit(true);
}

/// Without an edit in the window the late notification changes nothing:
/// clean against the loaded preset. An edit from before the load is gone
/// with it.
fn own_load_alone(link: bool) {
    let rig = rig(link);
    rig.core.set_input_preamp(1, 3.0).unwrap();
    rig.deliver();
    assert!(rig.core.preset_dirty());
    rig.hold();
    rig.core.load_preset(1).unwrap();
    rig.delivered_load();
    assert_eq!(rig.changes(), Vec::<String>::new());
    assert_eq!(preamp(&rig.core, 0), -4.0);
    assert_eq!(preamp(&rig.core, 1), 0.0);
}

#[test]
fn wp_cg33_own_load_without_an_edit_is_clean_usb() {
    own_load_alone(false);
}

#[test]
fn wp_cg33_own_load_without_an_edit_is_clean_link() {
    own_load_alone(true);
}

/// The device's controls (or another host) load while this client edits,
/// before the notification arrives: the device applies the edit on the
/// loaded preset, and it stays unsaved. The loaded preset's own changes
/// (input 0) are the new baseline.
fn foreign_load_then_edit(link: bool) {
    let rig = rig(link);
    rig.hold();
    rig.load_elsewhere(1);
    rig.core.set_input_preamp(1, -1.5).unwrap();
    rig.delivered_load();
    assert_eq!(preamp(&rig.core, 0), -4.0);
    assert_eq!(preamp(&rig.core, 1), -1.5);
    assert_eq!(rig.core.get_state().active_preset_slot, 1);
    let changes = rig.changes();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(changes[0].contains("0.0 dB → -1.5 dB"), "{changes:?}");
}

#[test]
fn wp_cg33_an_edit_after_a_foreign_load_stays_unsaved_usb() {
    foreign_load_then_edit(false);
}

#[test]
fn wp_cg33_an_edit_after_a_foreign_load_stays_unsaved_link() {
    foreign_load_then_edit(true);
}

/// Link-like timing: the notification is late enough for several edits of
/// this client's and another Link client's change (notified in the same
/// batch, after the load) to land first. All stay unsaved; an edit this
/// client made before the load is gone with the old preset.
#[test]
fn wp_cg33_a_long_window_keeps_every_later_change_link() {
    let rig = rig(true);
    rig.core.set_input_preamp(0, 3.0).unwrap();
    rig.deliver();
    rig.hold();
    rig.load_elsewhere(1);
    rig.core.set_input_preamp(1, -1.5).unwrap();
    rig.core.set_input_preamp(1, -2.5).unwrap();
    // An input's delay: no output twin.
    assert!(
        rig.remote
            .poke(g::OFF_DELAYS, &1.5f32.to_le_bytes(), SRC_UART)
    );
    rig.delivered_load();
    assert_eq!(preamp(&rig.core, 0), -4.0);
    let changes = rig.changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes[0].contains("0.0 dB → -2.5 dB"), "{changes:?}");
    assert!(changes[1].contains("delay: 0 ms → 1.5 ms"), "{changes:?}");
}

/// A re-read that fails is tried again later (WP C15's retry): edits made
/// before the retry succeeds stay unsaved too.
#[test]
fn wp_cg33_edits_before_a_retried_reread_stay_unsaved() {
    let rig = rig(false);
    rig.hold();
    rig.load_elsewhere(1);
    rig.core.set_input_preamp(1, -1.5).unwrap();
    // The first re-read fails: nothing to do but wait for the retry.
    let fail = |on: bool| rig.gate.failing.store(on, Ordering::SeqCst);
    fail(true);
    let r = rig.deliver();
    assert_eq!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
    fail(false);
    rig.core.set_input_preamp(1, -3.0).unwrap();
    let r = rig
        .core
        .process_notifications_at(Instant::now() + REFRESH_FIRST_DELAY);
    assert_ne!(r.flags & NOTIFY_REFRESHED, 0, "{r:?}");
    assert_eq!(preamp(&rig.core, 0), -4.0);
    let changes = rig.changes();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(changes[0].contains("0.0 dB → -3.0 dB"), "{changes:?}");
}

/// The baseline is fixed byte run by byte run: a run the mirror changed
/// that the device does not hold (the load replaced it) is the re-read's,
/// one it holds is the old baseline's; images of another size are left as
/// read.
#[test]
fn wp_cg33_keep_edits_after_load_restores_only_held_runs() {
    let old = [1u8, 2, 3, 4, 5, 6];
    let mirror = [1u8, 9, 9, 4, 7, 6];
    let mut baseline = [0u8, 9, 9, 0, 8, 0];
    crate::presets::keep_edits_after_load(&mut baseline, &old, &mirror);
    assert_eq!(baseline, [0, 2, 3, 0, 8, 0]);
    let mut short = [9u8, 9];
    crate::presets::keep_edits_after_load(&mut short, &old, &mirror);
    assert_eq!(short, [9, 9]);
}
