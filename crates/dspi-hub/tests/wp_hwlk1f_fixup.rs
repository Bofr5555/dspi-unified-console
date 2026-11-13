//! WP HWLK1F, fixup hwlk1f: review hwlk1f's probes kept as tests, asserting the
//! fixed behaviour.
//!
//! P1 (F1): a unit WITH the audio link whose transfers fault (a bus error, Link
//!     status `ERROR` like a stall, but not a stall) is never taken for a
//!     firmware without the link: it keeps what it was last seen as, and its
//!     scene ops are never written at once. Without a last poll it is not told.
//! P2 (F4): an offline unit without the link is as last seen: its facts are
//!     kept across polls, and a scene's op for it is `no_device` at once.
//! P3 (F3): the core's align sentence when every write failed: nothing written.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dspi_hub::{Hub, HubConfig, Role};
use dspi_proto::ext::audio_link::{EXT_FRAME_COUNTER, EXT_LINK_INFO, LinkRole};
use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::AudioLinkSetup;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, InRead, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::{Value, json};

type Outs = Arc<Mutex<Vec<(u8, u16, Vec<u8>)>>>;

/// A link receiver whose `EXT_LINK_INFO` and `EXT_FRAME_COUNTER` reads fault
/// (a bus error, not a stall) while `faulting` is set.
struct Flaky {
    inner: FakeDevice,
    faulting: Arc<AtomicBool>,
    outs: Outs,
}

impl Flaky {
    fn fault(&self, o: u8, v: u16) -> bool {
        o == op::REQ_EXTENSION
            && (v == EXT_LINK_INFO || v == EXT_FRAME_COUNTER)
            && self.faulting.load(Ordering::SeqCst)
    }
}

impl Transport for Flaky {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.fault(o, v) {
            return Err(TransportError::Usb("transfer fault".into()));
        }
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.fault(o, v) {
            return Err(TransportError::Usb("transfer fault".into()));
        }
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        let r = self.inner.control_out(o, v, d);
        if r.is_ok() {
            self.outs.lock().unwrap().push((o, v, d.to_vec()));
        }
        r
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
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
    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>, TransportError>> {
        self.inner.control_in_batch(reads)
    }
}

fn model() -> Value {
    json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "fanOuts": [],
        "units": [
            {"id": "A", "name": "Main left", "serial": "HW-A", "room": "main", "upstream": "tx"},
            {"id": "F", "name": "Flaky right", "serial": "HW-F", "room": "main", "upstream": "tx"},
            {"id": "K", "name": "K340", "serial": "HW-K", "room": "main", "upstream": "tx"}
        ],
        "scenes": [
            {"id": "f1", "name": "F delay", "actions": [{"target": "unit:F", "op": "out_delay", "arg": 0, "value": 48}]},
            {"id": "allmute", "name": "All mute", "actions": [{"target": "all", "op": "mute"}]},
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]}
        ]
    })
}

fn hub() -> (Hub, Arc<AtomicBool>, Outs, u8) {
    let hub = Hub::start(HubConfig {
        network: None,
        initial_scan: false,
        ..HubConfig::in_process("fixup hwlk1f")
    })
    .unwrap();
    let at = |role| AudioLinkSetup {
        role,
        ..AudioLinkSetup::default()
    };
    hub.attach(
        Box::new(FakeDevice::demo_audio_link_as(at(LinkRole::Sender)).with_serial("HW-TX")),
        0,
    )
    .unwrap();
    hub.attach(
        Box::new(FakeDevice::demo_audio_link_as(at(LinkRole::Receiver)).with_serial("HW-A")),
        0,
    )
    .unwrap();
    let faulting = Arc::new(AtomicBool::new(false));
    let outs = Outs::default();
    hub.attach(
        Box::new(Flaky {
            inner: FakeDevice::demo_audio_link_as(at(LinkRole::Receiver)).with_serial("HW-F"),
            faulting: Arc::clone(&faulting),
            outs: Arc::clone(&outs),
        }),
        0,
    )
    .unwrap();
    let k = hub
        .attach(Box::new(FakeDevice::demo().with_serial("HW-K")), 0)
        .unwrap();
    hub.handle()
        .system_set_model(serde_json::from_value(model()).unwrap())
        .unwrap();
    (hub, faulting, outs, k)
}

/// P1: a bus fault on two reads keeps a link unit as last seen, and nothing
/// meant for its frame is written at once.
#[test]
fn fixup_hwlk1f_p1_a_transient_fault_is_never_taken_for_no_audio_link() {
    let (hub, faulting, outs, _) = hub();
    let h = hub.handle();
    let s = h.system_status();
    assert!(!s.observed["HW-F"].without_alink());
    assert!(
        s.observed["HW-K"].without_alink(),
        "a real stall still tells K"
    );

    faulting.store(true, Ordering::SeqCst);
    let s = h.system_status();
    assert_eq!(
        serde_json::to_value(&s.observed["HW-F"]).unwrap()["alink"],
        "present",
        "kept as last seen"
    );
    let issues: Vec<(Option<String>, String)> = s
        .alignment
        .issues
        .iter()
        .map(|i| (i.subject.clone(), i.code.clone()))
        .collect();
    assert!(
        !issues.contains(&(Some("F".into()), "no_alink".into())),
        "{issues:?}"
    );
    assert!(
        issues.contains(&(Some("K".into()), "no_alink".into())),
        "{issues:?}"
    );

    // F alone, no frame of its own: refused (nothing can be scheduled), never at once.
    let r = h.system_apply_scene("f1", Role::Admin);
    assert!(r.is_err(), "{r:?}");
    // Everyone: the sender's write; F not known to take it; K at once.
    let r2 = h.system_apply_scene("allmute", Role::Admin).unwrap();
    assert!(
        r2.plan.direct.iter().all(|o| o.element == "K"),
        "{:?}",
        r2.plan.direct
    );
    let f: Vec<_> = r2.writes.iter().filter(|w| w.element == "F").collect();
    assert_eq!(f.len(), 1, "{:?}", r2.writes);
    assert_eq!(
        (f[0].result.as_str(), f[0].reason.as_deref()),
        ("no_frame", Some("no link frame read"))
    );
    assert!(!r2.on_time);
    let at_once: Vec<_> = outs
        .lock()
        .unwrap()
        .iter()
        .filter(|(o, _, _)| *o == op::REQ_SET_OUTPUT_DELAY || *o == op::REQ_SET_USER_MUTE)
        .cloned()
        .collect();
    assert!(
        at_once.is_empty(),
        "written at once to a link unit: {at_once:?}"
    );

    faulting.store(false, Ordering::SeqCst);
    let s = h.system_status();
    assert!(!s.observed["HW-F"].without_alink());
}

/// P1 without a last poll: a fault is not told, never "no audio link".
#[test]
fn fixup_hwlk1f_p1_a_fault_before_any_poll_is_not_told() {
    let (hub, faulting, _, _) = hub();
    faulting.store(true, Ordering::SeqCst);
    let s = hub.handle().system_status();
    assert_eq!(s.observed["HW-F"].alink, None);
    assert!(
        !s.alignment
            .issues
            .iter()
            .any(|i| i.subject.as_deref() == Some("F") && i.code == "no_alink")
    );
}

/// P2: an offline unit without the link is remembered as such across polls.
#[test]
fn fixup_hwlk1f_p2_an_offline_plain_unit_is_as_last_seen() {
    let (hub, _, _, k) = hub();
    let h = hub.handle();
    let s = h.system_status();
    assert!(s.observed["HW-K"].without_alink());
    hub.detach(k).unwrap();
    let start = std::time::Instant::now();
    loop {
        let s = h.system_status();
        if !s.observed["HW-K"].online {
            assert!(s.observed["HW-K"].without_alink(), "as last seen");
            assert!(
                !s.alignment
                    .units
                    .iter()
                    .any(|p| p.unit == "K" && p.writable)
            );
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Two polls later, still.
    assert!(h.system_status().observed["HW-K"].without_alink());
    let r = h.system_apply_scene("evening", Role::Admin).unwrap();
    assert_eq!(r.plan.direct.len(), 1, "{:?}", r.plan);
    let kw: Vec<_> = r.writes.iter().filter(|w| w.element == "K").collect();
    assert_eq!(kw.len(), 1, "{:?}", r.writes);
    assert_eq!(kw[0].result, "no_device");
}

/// P3: every write failed, nothing written: said so.
#[test]
fn fixup_hwlk1f_p3_align_sentence_when_every_write_failed() {
    let model = json!({"units": [{"id": "A", "name": "Main left"}, {"id": "B", "name": "Main right"}, {"id": "K", "name": "K340"}]});
    let report = json!({
        "applied": true,
        "alignment": {"issues": []},
        "writes": [
            {"serial": "HW-A", "element": "A", "what": "phase", "result": "timeout"},
            {"serial": "HW-B", "element": "B", "what": "phase", "result": "refused"},
            {"serial": "HW-K", "element": "K", "what": "phase", "result": "skipped", "reason": "no audio link (firmware)"}
        ]
    });
    let out = dspi_core::link::system::system_align_outcome(&report, &model);
    assert_eq!(
        out.message,
        "Nothing was written: 2 writes failed; skipped on K340 (no audio link (firmware))."
    );
}
