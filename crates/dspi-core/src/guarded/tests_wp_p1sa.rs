//! WP P1SA tests: the guarded apply on the demo device. Keep keeps; the
//! timeout, Revert and a failed apply put every value back (bulk and the
//! settings outside the image); a second begin is refused; a lost USB
//! connection writes nothing and replays nothing, and Revert works on the
//! user's word once the device is back. S7's part: an import's hardware part,
//! output enables and a control-surface edit cut part way end applied or
//! reverted, never half.
//!
//! The timeout runs on this thread's guard clock ([`super::skew`]). Stores
//! live under `std::env::temp_dir()`, which `ci/run.sh` points at the home
//! volume (never tmpfs).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use dspi_proto::generated::limiter;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::{Value, json};

use super::*;
use crate::records::{DspState, FilterParams};
use crate::consts::{IMPORT_ALL_BLOCKS, IMPORT_BY_BLOCK, IMPORT_LIMITERS};

/// No budget: every OUT transfer goes through.
const UNLIMITED: usize = usize::MAX;

/// The demo device behind a gate: it counts the OUT transfers that reach the
/// device; with `gone` set every transfer fails as an unplugged device's
/// does; `budget` OUT transfers pass, then each one is refused (stalled)
/// without reaching the device, until the budget is lifted.
struct Gate {
    dev: FakeDevice,
    outs: Arc<AtomicUsize>,
    gone: Arc<AtomicBool>,
    budget: Arc<AtomicUsize>,
    refuse: Arc<AtomicUsize>,
}

impl Transport for Gate {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.gone.load(Ordering::SeqCst) {
            return Err(TransportError::Disconnected);
        }
        self.dev.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if self.gone.load(Ordering::SeqCst) {
            return Err(TransportError::Disconnected);
        }
        self.dev.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        if self.gone.load(Ordering::SeqCst) {
            return Err(TransportError::Disconnected);
        }
        let refuse = self.refuse.load(Ordering::SeqCst);
        if refuse > 0 {
            self.refuse.store(refuse - 1, Ordering::SeqCst);
            return Err(TransportError::Stalled { opcode: op });
        }
        let left = self.budget.load(Ordering::SeqCst);
        if left == 0 {
            return Err(TransportError::Stalled { opcode: op });
        }
        if left != UNLIMITED {
            self.budget.store(left - 1, Ordering::SeqCst);
        }
        self.outs.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(op, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

struct Rig {
    core: Arc<DspiCore>,
    remote: FakeRemote,
    outs: Arc<AtomicUsize>,
    gone: Arc<AtomicBool>,
    budget: Arc<AtomicUsize>,
    refuse: Arc<AtomicUsize>,
}

impl Rig {
    fn sent(&self) -> usize {
        self.outs.load(Ordering::SeqCst)
    }
    /// Let `n` more OUT transfers through, then refuse the rest.
    fn allow(&self, n: usize) {
        self.budget.store(n, Ordering::SeqCst);
    }
    fn lift(&self) {
        self.budget.store(UNLIMITED, Ordering::SeqCst);
    }
    /// Refuse the next `n` OUT transfers, then let them through again.
    fn refuse_next(&self, n: usize) {
        self.refuse.store(n, Ordering::SeqCst);
    }
}

fn rig() -> Rig {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let outs = Arc::new(AtomicUsize::new(0));
    let gone = Arc::new(AtomicBool::new(false));
    let budget = Arc::new(AtomicUsize::new(UNLIMITED));
    let refuse = Arc::new(AtomicUsize::new(0));
    let gate = Gate {
        dev,
        outs: Arc::clone(&outs),
        gone: Arc::clone(&gone),
        budget: Arc::clone(&budget),
        refuse: Arc::clone(&refuse),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(gate), "P1SA-DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core.cs_fetch().unwrap();
    Rig {
        core,
        remote,
        outs,
        gone,
        budget,
        refuse,
    }
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// Move the guard clock past `s` seconds.
fn after(s: u64) {
    skew::advance(Duration::from_secs(s) + Duration::from_millis(1));
}

fn latency(core: &DspiCore) -> u8 {
    let l = core.fetch_output_latency().unwrap();
    let f = l.fields.iter().find(|f| f.name == "requested_id").unwrap();
    f.value.parse().unwrap()
}

/// Everything a guard puts back, read from the device: the mirror, the image
/// on the device, and the settings outside it.
#[derive(Debug, PartialEq)]
struct Whole {
    state: DspState,
    image: Vec<u8>,
    device_name: String,
    source_name: String,
    latency: u8,
    surfaces: Value,
}

fn whole(r: &Rig) -> Whole {
    let core = &r.core;
    core.fetch_all().unwrap();
    core.cs_fetch().unwrap();
    let mut cs = json_of(&core.cs_snapshot());
    // The configuration, not live readings (the display's status, its page).
    if let Some(o) = cs.as_object_mut() {
        o.retain(|k, _| {
            matches!(
                k.as_str(),
                "bindings" | "names" | "ir" | "groups" | "macros" | "display"
            )
        });
    }
    if let Some(d) = cs.get_mut("display").and_then(Value::as_object_mut) {
        d.retain(|k, _| matches!(k.as_str(), "config" | "pages"));
    }
    Whole {
        state: core.get_state(),
        image: r.remote.image(),
        device_name: core.get_device_name().unwrap(),
        source_name: core.get_input_source_name(0).unwrap(),
        latency: latency(core),
        surfaces: cs,
    }
}

fn cs_ok(core: &DspiCore, op: Value) {
    let r = json_of(&core.cs_apply(op.to_string()));
    assert_eq!(r["ok"], true, "{op}: {r}");
}

/// A control-surface group renamed: one CS operation.
fn rename_group(core: &DspiCore, name: &str) -> Value {
    let s = json_of(&core.cs_snapshot());
    let mut g = s["groups"][0].clone();
    g["name"] = json!(name);
    json!({ "op": "group", "index": 0, "group": g })
}

/// Change the image (preamp, an EQ band, an output gain) and outside it (the
/// device's and a source's name, the output latency, a CS group).
fn change_everything(core: &DspiCore) {
    core.set_input_preamp(0, -7.5).unwrap();
    core.set_filter(
        0,
        2,
        FilterParams {
            filter_type: 1,
            bypass: false,
            freq: 1234.0,
            q: 2.0,
            gain: -4.0,
            ..core.get_state().filters[0][2]
        },
    )
    .unwrap();
    core.set_output_gain(1, -3.0).unwrap();
    core.set_device_name("Changed".into()).unwrap();
    core.set_input_source_name(0, "Other".into()).unwrap();
    let was = latency(core);
    core.set_output_latency(if was == 3 { 4 } else { 3 })
        .unwrap();
    cs_ok(core, rename_group(core, "Moved group"));
}

/// A document holding other EQ, preamp and gain than the device has; the
/// device is left as it was.
fn changed_document(core: &DspiCore) -> String {
    let back = core
        .snapshot_capture_document("back".into(), String::new())
        .unwrap();
    core.set_input_preamp(0, -6.0).unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    let doc = core
        .config_export("Kväll ☾".into(), "wp-p1sa".into(), 0)
        .unwrap();
    core.snapshot_recall_document(back).unwrap();
    doc
}

const ALL: u32 = IMPORT_BY_BLOCK | IMPORT_ALL_BLOCKS;

/// Keep keeps: the change stays past the timeout, nothing is written back.
#[test]
fn wp_p1sa_confirm_keeps() {
    let r = rig();
    let core = &r.core;
    let doc = changed_document(core);
    let s = core.apply_guarded_import(doc, ALL, u32::MAX, 0).unwrap();
    assert_eq!(s.phase, GuardPhase::Waiting);
    assert_eq!(s.label, "Kväll ☾");
    assert_eq!(s.timeout_ms, 30_000);
    assert!(s.remaining_ms > 29_000 && s.remaining_ms <= 30_000, "{s:?}");
    assert!(!s.hub);
    assert!(
        s.changes.iter().any(|c| c.contains("reamp")),
        "{:?}",
        s.changes
    );
    assert_eq!(core.get_state().input_preamp_db[0], -6.0);
    assert_eq!(core.apply_status(), s);

    let k = core.apply_confirm().unwrap();
    assert_eq!((k.phase, k.reason.as_str()), (GuardPhase::Kept, "confirmed"));
    assert_eq!(k.remaining_ms, 0);
    let sent = r.sent();
    after(60);
    assert_eq!(core.apply_poll(), k);
    assert_eq!(r.sent(), sent, "nothing written after Keep");
    assert_eq!(core.get_state().input_preamp_db[0], -6.0);
    assert_eq!(
        core.apply_confirm().unwrap_err(),
        DspiError::InvalidArgument {
            msg: NOTHING_WAITING.into()
        }
    );
}

/// The timeout reverts a flow on the caller's poll: every value back, bulk
/// and outside the image, the undo history ended.
#[test]
fn wp_p1sa_timeout_reverts_every_value() {
    let r = rig();
    let core = &r.core;
    let before = whole(&r);
    let s = core.apply_guard_begin("Tuning".into(), 5).unwrap();
    assert_eq!((s.phase, s.timeout_ms), (GuardPhase::Waiting, 5_000));
    change_everything(core);
    assert_ne!(whole(&r), before);
    // Before the deadline: the poll writes nothing.
    let sent = r.sent();
    after(4);
    assert_eq!(core.apply_poll().phase, GuardPhase::Waiting);
    assert_eq!(r.sent(), sent);
    after(1);
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "timeout"),
        "{end:?}"
    );
    assert!(end.message.contains("5 s"), "{}", end.message);
    assert_eq!(whole(&r), before);
    assert!(!core.can_undo(), "the history before a revert ends");
    assert_eq!(core.apply_status(), end);
}

/// Revert puts a guarded snapshot recall back; a second begin is refused
/// while one waits; a Keep after the timeout reverts.
#[test]
fn wp_p1sa_revert_busy_and_a_late_keep() {
    let r = rig();
    let core = &r.core;
    let dir = std::env::temp_dir().join("wp-p1sa-core").join(format!(
        "{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    core.snapshot_set_dir(dir.display().to_string()).unwrap();
    core.set_input_preamp(0, -11.0).unwrap();
    let snap = core.snapshot_capture("Låg".into(), String::new()).unwrap();
    core.set_input_preamp(0, 0.0).unwrap();
    let before = whole(&r);

    let s = core.apply_guarded_snapshot(snap.id.clone(), 30).unwrap();
    assert_eq!((s.phase, s.label.as_str()), (GuardPhase::Waiting, "Låg"));
    assert_eq!(core.get_state().input_preamp_db[0], -11.0);
    let sent = r.sent();
    assert_eq!(
        core.apply_guard_begin("other".into(), 10).unwrap_err(),
        DspiError::InvalidArgument { msg: BUSY.into() }
    );
    assert_eq!(
        core.apply_guarded_snapshot(snap.id.clone(), 10)
            .unwrap_err(),
        DspiError::InvalidArgument { msg: BUSY.into() }
    );
    assert_eq!(r.sent(), sent, "a refused begin sends nothing");
    let end = core.apply_revert().unwrap();
    assert_eq!((end.phase, end.reason.as_str()), (GuardPhase::Reverted, "reverted"));
    assert_eq!(whole(&r), before);

    // A Keep after the timeout reverts instead.
    core.apply_guarded_snapshot(snap.id, 30).unwrap();
    after(31);
    let late = core.apply_confirm().unwrap();
    assert_eq!((late.phase, late.reason.as_str()), (GuardPhase::Reverted, "timeout"));
    assert_eq!(whole(&r), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What is refused sends nothing and leaves no guard behind: not a
/// document, an empty selection, a document that is not a snapshot, a
/// snapshot of another device shape.
#[test]
fn wp_p1sa_refusals_leave_no_guard() {
    let r = rig();
    let core = &r.core;
    let sent = r.sent();
    assert!(matches!(
        core.apply_guarded_import("{".into(), ALL, u32::MAX, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    let doc = changed_document(core);
    let sent_doc = r.sent();
    assert!(sent_doc > sent);
    assert!(matches!(
        core.apply_guarded_import(doc.clone(), IMPORT_BY_BLOCK, u32::MAX, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.apply_guarded_snapshot_document(doc, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    // Another channel count: the recall refuses before sending.
    let mut snap = json_of(
        &core
            .snapshot_capture_document("x".into(), String::new())
            .unwrap(),
    );
    snap["meta"]["outputChannelCount"] = json!(99);
    let e = core
        .apply_guarded_snapshot_document(snap.to_string(), 0)
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    assert_eq!(r.sent(), sent_doc, "nothing sent");
    assert_eq!(core.apply_status(), GuardedApply::default());
    // Nothing waits: Revert has nothing to do.
    assert!(core.apply_revert().is_err());
}

/// An import whose bulk write fails is put back at once (S7: an import is
/// never left half done).
#[test]
fn wp_p1sa_a_failed_import_reverts_at_once() {
    let r = rig();
    let core = &r.core;
    let doc = changed_document(core);
    let before = whole(&r);
    // The import's bulk write is refused; the revert's goes through.
    r.refuse_next(1);
    let e = core.apply_guarded_import(doc, ALL, u32::MAX, 30).unwrap_err();
    assert!(matches!(e, DspiError::Transfer { .. }), "{e:?}");
    let s = core.apply_status();
    assert_eq!(
        (s.phase, s.reason.as_str()),
        (GuardPhase::Reverted, "failed"),
        "{s:?}"
    );
    assert_eq!(whole(&r), before);
}

/// How many OUT transfers `flow` makes.
fn outs_of(r: &Rig, flow: impl FnOnce()) -> usize {
    let n = r.sent();
    flow();
    r.sent() - n
}

/// S7: the hardware part of an import (limiters, written one by one while
/// the device keeps its output configuration apart from presets) cut part
/// way: Waiting with what was not written in the notes, then the timeout
/// puts every value back.
#[test]
fn wp_p1sa_s7_an_import_hardware_part_cut_part_way() {
    let r = rig();
    let core = &r.core;
    core.set_output_config_mode(0).unwrap();
    let back = core
        .snapshot_capture_document("back".into(), String::new())
        .unwrap();
    let t = limiter::LIMITER_PARAM_THRESHOLD_DB as u8;
    let e = limiter::LIMITER_PARAM_ENABLED as u8;
    for out in 0..2 {
        core.set_limiter_param(out, e, 1.0).unwrap();
        core.set_limiter_param(out, t, -12.0).unwrap();
    }
    let doc = core.config_export("Lim".into(), "wp-p1sa".into(), 0).unwrap();
    core.snapshot_recall_document(back).unwrap();
    let before = whole(&r);
    let opts = IMPORT_BY_BLOCK | IMPORT_LIMITERS;

    // The whole flow's writes, once, kept and undone by hand.
    let n = outs_of(&r, || {
        core.apply_guarded_import(doc.clone(), opts, u32::MAX, 30)
            .unwrap();
    });
    assert!(n >= 3, "{n}");
    core.apply_revert().unwrap();
    assert_eq!(whole(&r), before);

    // Cut before the last write.
    r.allow(n - 1);
    let s = core.apply_guarded_import(doc, opts, u32::MAX, 30).unwrap();
    r.lift();
    assert_eq!(s.phase, GuardPhase::Waiting);
    assert!(
        s.notes.iter().any(|l| l.contains("limiter")),
        "{:?}",
        s.notes
    );
    assert_ne!(whole(&r), before, "part applied");
    after(31);
    assert_eq!(core.apply_poll().phase, GuardPhase::Reverted);
    assert_eq!(whole(&r), before);
}

/// S7: output enables, one written and the next cut: Revert puts both back.
#[test]
fn wp_p1sa_s7_output_enables_cut_part_way() {
    let r = rig();
    let core = &r.core;
    let before = whole(&r);
    core.apply_guard_begin("Outputs off".into(), 30).unwrap();
    core.set_output_enable(0, false).unwrap();
    r.allow(0);
    assert!(core.set_output_enable(1, false).is_err());
    r.lift();
    assert_ne!(whole(&r), before);
    let end = core.apply_revert().unwrap();
    assert_eq!(end.phase, GuardPhase::Reverted);
    assert_eq!(whole(&r), before);
}

/// S7: a control-surface edit, one operation written and the next cut: the
/// timeout puts the configuration back.
#[test]
fn wp_p1sa_s7_a_control_surface_apply_cut_part_way() {
    let r = rig();
    let core = &r.core;
    let before = whole(&r);
    core.apply_guard_begin("Surfaces".into(), 10).unwrap();
    cs_ok(core, rename_group(core, "Ny grupp"));
    r.allow(0);
    let op = json!({ "op": "name", "index": 0, "name": "Knob" });
    let reply = json_of(&core.cs_apply(op.to_string()));
    assert_ne!(reply["ok"], true, "{reply}");
    r.lift();
    assert_ne!(whole(&r), before);
    after(11);
    assert_eq!(core.apply_poll().phase, GuardPhase::Reverted);
    assert_eq!(whole(&r), before);
}

/// A lost USB connection writes nothing and replays nothing; Revert, on the
/// user's word, puts the settings back once the same device is open again.
#[test]
fn wp_p1sa_a_lost_usb_connection_writes_nothing() {
    let r = rig();
    let core = &r.core;
    let before = whole(&r);
    let doc = changed_document(core);
    core.apply_guarded_import(doc, ALL, u32::MAX, 30).unwrap();
    let applied = r.remote.image();
    let sent = r.sent();
    r.gone.store(true, Ordering::SeqCst);
    after(31);
    let lost = core.apply_poll();
    assert_eq!(
        (lost.phase, lost.reason.as_str()),
        (GuardPhase::Lost, "connection_lost"),
        "{lost:?}"
    );
    assert!(lost.message.contains("may still hold"), "{}", lost.message);
    assert_eq!(r.sent(), sent);
    // Back: nothing was queued.
    r.gone.store(false, Ordering::SeqCst);
    assert_eq!(core.apply_poll(), lost);
    core.fetch_status().ok();
    assert_eq!(r.sent(), sent);
    assert_eq!(r.remote.image(), applied);
    // The user's Revert.
    let end = core.apply_revert().unwrap();
    assert_eq!(end.phase, GuardPhase::Reverted);
    assert_eq!(whole(&r), before);
    assert!(core.apply_revert().is_err(), "once");

    // A closed connection is lost at once, and nothing is sent after it.
    core.apply_guard_begin("x".into(), 30).unwrap();
    core.disconnect();
    assert_eq!(core.apply_poll().phase, GuardPhase::Lost);
    assert_eq!(core.apply_revert().unwrap_err().to_string().is_empty(), false);
}

/// The guard's default: off the hub, the user's choice.
#[test]
fn wp_p1sa_default_off_the_hub() {
    let r = rig();
    assert!(!r.core.apply_guard_default());
    assert_eq!(timeout_of(0), Duration::from_secs(30));
    assert_eq!(timeout_of(10_000), Duration::from_secs(590));
}
