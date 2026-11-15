//! WP ECHO1: the late own echo after a whole image (review p1efix F2). An own
//! `PARAM_CHANGED` whose write was sent before the last whole-image write (a
//! snapshot recall, an A/B side, an import, a preset load, a factory reset)
//! is not applied to the mirror, over a transport that holds the echo back
//! until the image is written (USB-like, MIDI-like and hub-like); a foreign
//! change in the meantime still applies, and an own write after the image
//! is handled as before. Adopts the reviewer's probe
//! (`review_p1efix_probe_a_late_own_echo_after_the_image`).
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dspi_transport::{
    Attributed, DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport,
    TransportCaps,
};

use super::*;

const SRC_HOST: u8 = nfy::PARAM_SRC_HOST_SET as u8;
const SRC_MIDI: u8 = nfy::PARAM_SRC_MIDI as u8;
const SRC_GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;
/// Our session on the hub-like link.
const OURS: u16 = 7;

/// The demo device behind a link of `kind` whose notifications can be held
/// back (`held`) and are then delivered in order: the echo of a write made
/// just before an image arrives only after it, as over MIDI or a hub.
/// MIDI-like: our writes come back tagged `MIDI`. Hub-like: the hub's origin
/// is our session for our writes and 0 (the device) for the rest; bulk goes
/// single-shot (`0xA1`).
struct Delayed {
    inner: FakeDevice,
    kind: TransportKind,
    held: Arc<AtomicBool>,
}

struct DelayedSource {
    inner: Box<dyn NotificationSource>,
    kind: TransportKind,
    held: Arc<AtomicBool>,
    queue: VecDeque<Vec<u8>>,
}

impl DelayedSource {
    /// The packet as this link carries it.
    fn carried(&self, mut p: Vec<u8>) -> Attributed {
        let v2 = p.len() >= 4 && p.first() == Some(&2);
        let id = p.get(1).copied().unwrap_or(0);
        let at = match id {
            x if x == nfy::NOTIFY_EVT_PARAM_CHANGED as u8 => 8,
            x if x == nfy::NOTIFY_EVT_BULK_INVALIDATED as u8 => 4,
            _ => usize::MAX,
        };
        let src = if v2 { p.get(at).copied() } else { None };
        if self.kind == TransportKind::Midi
            && id == nfy::NOTIFY_EVT_PARAM_CHANGED as u8
            && src == Some(SRC_HOST)
            && let Some(b) = p.get_mut(at)
        {
            *b = SRC_MIDI;
        }
        let origin = (self.kind == TransportKind::Link).then_some(match src {
            Some(s) if s == SRC_GPIO => 0,
            Some(_) => OURS,
            None => 0,
        });
        Attributed { packet: p, origin }
    }
}

impl NotificationSource for DelayedSource {
    fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
        Ok(self.read_attributed(timeout)?.packet)
    }

    fn read_attributed(&mut self, timeout: Duration) -> dspi_transport::Result<Attributed> {
        let silence = Attributed {
            packet: Vec::new(),
            origin: None,
        };
        if !self.held.load(Ordering::SeqCst)
            && let Some(p) = self.queue.pop_front()
        {
            return Ok(self.carried(p));
        }
        let p = self.inner.read(timeout.min(Duration::from_millis(10)))?;
        if p.is_empty() {
            return Ok(silence);
        }
        if self.held.load(Ordering::SeqCst) || !self.queue.is_empty() {
            self.queue.push_back(p);
            return Ok(silence);
        }
        Ok(self.carried(p))
    }
}

impl Transport for Delayed {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in_upto(opcode, value, len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        Some(Box::new(DelayedSource {
            inner: self.inner.notifications()?,
            kind: self.kind,
            held: Arc::clone(&self.held),
            queue: VecDeque::new(),
        }))
    }
    fn caps(&self) -> TransportCaps {
        let link = self.kind == TransportKind::Link;
        TransportCaps {
            kind: self.kind,
            chunked_bulk: !link,
            remote: link,
        }
    }
    fn session_id(&self) -> Option<u16> {
        (self.kind == TransportKind::Link).then_some(OURS)
    }
}

/// Pump until ten quiet rounds in a row, every result ORed.
fn quiet(core: &DspiCore) -> NotifyResult {
    let mut acc = NotifyResult::default();
    let mut q = 0;
    while q < 10 {
        std::thread::sleep(Duration::from_millis(10));
        let r = core.process_notifications();
        if r == NotifyResult::default() {
            q += 1;
        } else {
            q = 0;
        }
        acc.flags |= r.flags;
        acc.filter_channels |= r.filter_channels;
    }
    acc
}

const KINDS: [TransportKind; 3] = [
    TransportKind::Fake,
    TransportKind::Midi,
    TransportKind::Link,
];

fn open(kind: TransportKind) -> (Arc<DspiCore>, FakeRemote, Arc<AtomicBool>) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let held = Arc::new(AtomicBool::new(false));
    let core = DspiCore::new();
    let t = Delayed {
        inner: dev,
        kind,
        held: Arc::clone(&held),
    };
    core.attach_probed(Box::new(t), "DEMO".into(), 0).unwrap();
    quiet(&core);
    assert_eq!(core.lock().conn.as_ref().unwrap().caps.kind, kind);
    (core, remote, held)
}

fn vol(core: &DspiCore) -> f32 {
    core.get_state().master_volume_db
}

/// What the device holds: the mirror after a re-read.
fn device_vol(core: &DspiCore) -> f32 {
    core.refresh_params().unwrap();
    vol(core)
}

/// A master-volume `PARAM_CHANGED` with `source` (the fake renumbers it).
fn volume_packet(source: u8, db: f32) -> Vec<u8> {
    let mut p = vec![2, nfy::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
    p.extend((g::OFF_MASTER_VOLUME as u16).to_le_bytes());
    p.extend(4u16.to_le_bytes());
    p.extend([source, 0, 0, 0]);
    p.extend(db.to_le_bytes());
    p
}

/// B at `v0 - 3`, the device and the mirror back at `v0`.
fn with_b(core: &DspiCore) -> (f32, String) {
    let v0 = vol(core);
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(core);
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(core);
    (v0, b)
}

#[test]
fn wp_echo1_a_late_own_echo_after_a_recall_is_not_applied() {
    for kind in KINDS {
        let (core, _remote, held) = open(kind);
        let (v0, b) = with_b(&core);
        // A write just before the recall; its echo is still on its way.
        held.store(true, Ordering::SeqCst);
        core.set_master_volume(v0 - 0.5).unwrap();
        core.snapshot_recall_document(b).unwrap();
        assert_eq!(vol(&core), v0 - 3.0, "{kind:?}: B applied");
        held.store(false, Ordering::SeqCst);
        quiet(&core);
        assert_eq!(
            vol(&core),
            v0 - 3.0,
            "{kind:?}: the echo of the write before the image must not undo it"
        );
        assert_eq!(device_vol(&core), v0 - 3.0, "{kind:?}: the device holds B");
    }
}

#[test]
fn wp_echo1_a_foreign_change_during_a_recall_still_applies() {
    for kind in KINDS {
        let (core, remote, held) = open(kind);
        let (v0, b) = with_b(&core);
        held.store(true, Ordering::SeqCst);
        core.set_master_volume(v0 - 0.5).unwrap();
        core.snapshot_recall_document(b).unwrap();
        // A control surface turns the knob while the echo is held back.
        remote.push(volume_packet(SRC_GPIO, -30.0));
        std::thread::sleep(Duration::from_millis(50));
        held.store(false, Ordering::SeqCst);
        let r = quiet(&core);
        assert_eq!(vol(&core), -30.0, "{kind:?}: the foreign change applies");
        assert_ne!(r.flags & NOTIFY_STATE, 0, "{kind:?}: and is reported");
    }
}

#[test]
fn wp_echo1_an_own_write_after_the_image_behaves_as_before() {
    for kind in KINDS {
        let (core, remote, held) = open(kind);
        let (v0, b) = with_b(&core);
        core.snapshot_recall_document(b.clone()).unwrap();
        quiet(&core);
        // Its own echo, held back, is silent and lands.
        held.store(true, Ordering::SeqCst);
        core.set_master_volume(v0 - 6.0).unwrap();
        held.store(false, Ordering::SeqCst);
        let r = quiet(&core);
        assert_eq!(vol(&core), v0 - 6.0, "{kind:?}");
        assert_eq!(
            r.flags & NOTIFY_STATE,
            0,
            "{kind:?}: an unchanged own echo is silent"
        );
        // An own echo the device changed (clamped) is applied and reported.
        remote.push(volume_packet(SRC_HOST, v0 - 7.0));
        let r = quiet(&core);
        assert_eq!(vol(&core), v0 - 7.0, "{kind:?}: the device's value wins");
        assert_ne!(r.flags & NOTIFY_STATE, 0, "{kind:?}");

        // A write before the image and one after it, both held back: the
        // last one is what the mirror and the device end on.
        held.store(true, Ordering::SeqCst);
        core.set_master_volume(v0 - 1.0).unwrap();
        core.snapshot_recall_document(b).unwrap();
        core.set_master_volume(v0 - 9.0).unwrap();
        held.store(false, Ordering::SeqCst);
        quiet(&core);
        assert_eq!(vol(&core), v0 - 9.0, "{kind:?}");
        assert_eq!(device_vol(&core), v0 - 9.0, "{kind:?}");
    }
}

#[test]
fn wp_echo1_an_ab_toggle_and_an_import_mark_the_image_too() {
    let (core, _remote, held) = open(TransportKind::Midi);
    let (v0, b) = with_b(&core);
    core.ab_start_documents(String::new(), b.clone()).unwrap();
    held.store(true, Ordering::SeqCst);
    core.set_master_volume(v0 - 0.5).unwrap(); // on A; its echo is late
    core.ab_toggle().unwrap(); // to B
    held.store(false, Ordering::SeqCst);
    quiet(&core);
    assert_eq!(vol(&core), v0 - 3.0, "A/B: B holds");
    assert_eq!(device_vol(&core), v0 - 3.0);

    let (core, _remote, held) = open(TransportKind::Link);
    let v0 = vol(&core);
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    let doc = core.config_export("B".into(), "test".into(), 0).unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    held.store(true, Ordering::SeqCst);
    core.set_master_volume(v0 - 0.5).unwrap();
    let reply: serde_json::Value =
        serde_json::from_str(&core.config_import_tok(doc, IMPORT_ALL_BLOCKS)).unwrap();
    assert_eq!(reply["ok"], serde_json::json!(true), "{reply}");
    held.store(false, Ordering::SeqCst);
    quiet(&core);
    assert_eq!(vol(&core), v0 - 3.0, "import: the file's volume holds");
    assert_eq!(device_vol(&core), v0 - 3.0);
}

#[test]
fn wp_echo1_a_preset_load_and_a_factory_reset_mark_the_image() {
    let (core, remote, _held) = open(TransportKind::Fake);
    core.set_master_volume_mode(1).unwrap(); // saved with presets
    let v0 = vol(&core);
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    core.save_preset_tok(1).unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    core.load_preset(1).unwrap();
    quiet(&core);
    assert_eq!(vol(&core), v0 - 3.0, "the preset's volume");
    // The echo of the write before the load, arriving only now.
    remote.push(volume_packet(SRC_HOST, v0));
    quiet(&core);
    assert_eq!(
        vol(&core),
        v0 - 3.0,
        "preset load: the late echo is dropped"
    );
    assert_eq!(device_vol(&core), v0 - 3.0);

    core.set_master_volume(v0 - 4.0).unwrap();
    quiet(&core);
    core.factory_reset_tok().unwrap();
    quiet(&core);
    let reset = vol(&core);
    remote.push(volume_packet(SRC_HOST, v0 - 4.0));
    quiet(&core);
    assert_eq!(vol(&core), reset, "factory reset: the late echo is dropped");
    assert_eq!(device_vol(&core), reset);
}

/// The reviewer's probe (review p1efix F2, `probe4-late-echo.log`), adopted:
/// B recalled, then the echo of this console's earlier write arrives.
#[test]
fn wp_echo1_review_p1efix_probe_a_late_own_echo_after_the_image() {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    quiet(&core);
    let v0 = core.get_state().master_volume_db;
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core); // the real echo of v0 is applied; the mirror and device hold v0
    core.snapshot_recall_document(b).unwrap();
    assert_eq!(core.get_state().master_volume_db, v0 - 3.0, "B applied");
    // The same echo, had it arrived only now (its bytes: our own write of v0).
    remote.push(volume_packet(SRC_HOST, v0));
    std::thread::sleep(Duration::from_millis(100));
    core.process_notifications();
    let mirror = core.get_state().master_volume_db;
    core.refresh_params().unwrap();
    assert_eq!(core.get_state().master_volume_db, v0 - 3.0, "device");
    assert_eq!(
        mirror,
        v0 - 3.0,
        "a late own echo must not put the old value over the image"
    );
}

/// The rule ends: after [`EchoFilter::LATE_ECHO`] with no write of ours, an
/// own-looking change (another app on the same USB host) applies again.
#[test]
fn wp_echo1_after_the_late_echo_window_an_own_looking_change_applies() {
    let (core, remote, _held) = open(TransportKind::Fake);
    let (v0, b) = with_b(&core);
    core.snapshot_recall_document(b).unwrap();
    quiet(&core);
    // Move the mark back past the window instead of sleeping five seconds.
    {
        let mut inner = core.lock();
        let echo = &mut inner.conn.as_mut().unwrap().echo;
        let past = dspi_transport::clock::Instant::now()
            .checked_sub(dspi_session::notify::EchoFilter::LATE_ECHO + Duration::from_secs(1))
            .unwrap();
        echo.whole_image_sent(past);
        echo.whole_image_done(past);
    }
    remote.push(volume_packet(SRC_HOST, v0 - 8.0));
    quiet(&core);
    assert_eq!(vol(&core), v0 - 8.0);
}

// -- review batch1 F1 -- the analyser worker takes a Writable every tick
// without writing a parameter; it must not end the late-echo window. The
// reviewer's probe (`probe-echo1-analyser-tests_review_batch1.rs`), adopted.

/// A write, then a recall whose image lands before that write's echo, with
/// the analyser `analyser` (started, a request `active`) and `gap_ms`
/// between the image and the echo. The kinds whose mirror ends wrong.
fn late_echo_with_the_analyser(analyser: bool, active: bool, gap_ms: u64) -> Vec<String> {
    let mut bad = Vec::new();
    for kind in KINDS {
        let (core, _remote, held) = open(kind);
        if analyser {
            core.rta_start();
            if active {
                core.rta_set_request(crate::RtaRequest {
                    tap: 1,
                    mask: 0b01,
                    wants_bins: false,
                    fft_order: 10,
                    avg_ms: 300,
                    peak_decay_db_s: 12,
                    watching: false,
                });
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        let (v0, b) = with_b(&core);
        held.store(true, Ordering::SeqCst);
        core.set_master_volume(v0 - 0.5).unwrap();
        core.snapshot_recall_document(b).unwrap();
        assert_eq!(vol(&core), v0 - 3.0, "{kind:?}: B applied");
        std::thread::sleep(Duration::from_millis(gap_ms));
        held.store(false, Ordering::SeqCst);
        quiet(&core);
        if vol(&core) != v0 - 3.0 {
            bad.push(format!(
                "{kind:?}: mirror {} (device holds {})",
                vol(&core),
                v0 - 3.0
            ));
        }
    }
    bad
}

#[test]
fn wp_echo1_review_batch1_probe_control_no_analyser() {
    assert_eq!(
        late_echo_with_the_analyser(false, false, 1500),
        Vec::<String>::new()
    );
}

#[test]
fn wp_echo1_review_batch1_probe_analyser_idle() {
    assert_eq!(
        late_echo_with_the_analyser(true, false, 1500),
        Vec::<String>::new()
    );
}

#[test]
fn wp_echo1_review_batch1_probe_analyser_active() {
    assert_eq!(
        late_echo_with_the_analyser(true, true, 250),
        Vec::<String>::new()
    );
}

/// A raw preset load (the Terminal's `raw`) marks a whole image too.
#[test]
fn wp_echo1_a_raw_preset_load_marks_the_image() {
    let (core, remote, _held) = open(TransportKind::Fake);
    core.set_master_volume_mode(1).unwrap();
    let v0 = vol(&core);
    core.set_master_volume(v0 - 3.0).unwrap();
    quiet(&core);
    core.save_preset_tok(1).unwrap();
    core.set_master_volume(v0).unwrap();
    quiet(&core);
    core.raw_get(dspi_proto::generated::opcodes::REQ_PRESET_LOAD, 1, 1)
        .unwrap();
    quiet(&core); // the preset's invalidation re-reads the image
    assert_eq!(vol(&core), v0 - 3.0, "the preset's volume");
    remote.push(volume_packet(SRC_HOST, v0));
    quiet(&core);
    assert_eq!(vol(&core), v0 - 3.0, "the late echo is dropped");
}

/// Only a send stamps our write: taking a `Writable` does
/// not; `set`, `set_as_read`, `session` and `sending` do.
#[test]
fn wp_echo1_only_a_send_ends_the_late_echo_window() {
    use dspi_session::notify::{Event, Notification, Source};
    let (core, _remote, _held) = open(TransportKind::Fake);
    let late = |core: &DspiCore| {
        let note = Notification {
            seq: 0,
            event: Event::ParamChanged {
                offset: g::OFF_MASTER_VOLUME as u16,
                source: Source::HostSet,
                bytes: vec![0; 4],
            },
            lost: false,
            origin: None,
            received: dspi_transport::clock::Instant::now(),
        };
        let mut inner = core.lock();
        let conn = inner.conn().unwrap();
        conn.echo.is_late_own_echo(conn.caps.kind, None, &note)
    };
    let mark = |core: &DspiCore| {
        let now = dspi_transport::clock::Instant::now();
        let mut inner = core.lock();
        let echo = &mut inner.conn().unwrap().echo;
        echo.whole_image_sent(now);
        echo.whole_image_done(now);
    };
    use dspi_proto::generated::opcodes as op;
    type SendFn = fn(&mut crate::Writable<'_>);
    let sends: [(&str, SendFn); 4] = [
        ("set", |w| {
            w.set(op::REQ_SET_PREAMP, 0, &0f32.to_le_bytes()).unwrap();
        }),
        ("set_as_read", |w| {
            let _ = w.set_as_read(op::REQ_SAVE_PARAMS, 0);
        }),
        ("session", |w| {
            w.session().unwrap();
        }),
        ("sending", |w| {
            w.sending();
        }),
    ];
    for (what, send) in sends {
        mark(&core);
        let _ = core.lock().writable_conn().unwrap().conn();
        assert!(late(&core), "{what}: taking a Writable is no write");
        {
            let mut inner = core.lock();
            let mut w = inner.writable_conn().unwrap();
            send(&mut w);
        }
        std::thread::sleep(Duration::from_millis(2));
        assert!(!late(&core), "{what}: a write of ours ends it");
    }
}
