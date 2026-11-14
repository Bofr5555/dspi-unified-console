//! WP P1AB tests: A/B compare on the demo device.
//!
//! Stores live under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::Value;

use super::*;
use crate::records::FilterParams;

/// The demo device behind a gate: it counts the OUT transfers, and with
/// `gone` set every transfer fails as an unplugged device's does.
struct Gate {
    dev: FakeDevice,
    outs: Arc<AtomicUsize>,
    gone: Arc<AtomicBool>,
}

impl Gate {
    fn check(&self) -> dspi_transport::Result<()> {
        if self.gone.load(Ordering::SeqCst) {
            Err(TransportError::Disconnected)
        } else {
            Ok(())
        }
    }
}

impl Transport for Gate {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.check()?;
        self.dev.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.check()?;
        self.dev.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.check()?;
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
}

fn rig() -> Rig {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let outs = Arc::new(AtomicUsize::new(0));
    let gone = Arc::new(AtomicBool::new(false));
    let gate = Gate {
        dev,
        outs: Arc::clone(&outs),
        gone: Arc::clone(&gone),
    };
    let core = DspiCore::new();
    core.attach_probed(Box::new(gate), "P1AB-DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    Rig {
        core,
        remote,
        outs,
        gone,
    }
}

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1ab-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// Change values in the image (preamp, an EQ band, a channel name, an
/// output gain) and outside it (the device's name).
fn change(core: &DspiCore) {
    core.set_input_preamp(0, -6.5).unwrap();
    core.set_filter(
        0,
        1,
        FilterParams {
            filter_type: 1,
            bypass: false,
            freq: 2500.0,
            q: 1.5,
            gain: 3.0,
            ..core.get_state().filters[0][1]
        },
    )
    .unwrap();
    core.set_channel_name(0, "B-sidan ✓".into()).unwrap();
    core.set_output_gain(0, -2.0).unwrap();
    core.set_device_name("Bee".into()).unwrap();
}

/// A device on A, and B's snapshot document: A is the demo's state, B the
/// same after [`change`].
fn a_and_b(core: &DspiCore) -> (String, String) {
    let a = core
        .snapshot_capture_document("Start".into(), String::new())
        .unwrap();
    change(core);
    let b = core
        .snapshot_capture_document("Bright".into(), "B ☀".into())
        .unwrap();
    core.snapshot_recall_document(a.clone()).unwrap();
    (a, b)
}

fn recalls(core: &DspiCore) -> Vec<String> {
    core.event_log_read(String::new(), 0, 1000)
        .entries
        .into_iter()
        .filter(|e| e.kind == "recall")
        .map(|e| e.text)
        .collect()
}

/// Toggle twice returns to A exactly: the mirror, the device's image and
/// the setting outside it. Each toggle is a recall in the event log and
/// ends the undo history; it is no undo step.
#[test]
fn wp_p1ab_toggle_twice_returns_to_a_exactly() {
    let r = rig();
    let core = &r.core;
    let (_, b) = a_and_b(core);
    // An edit before the compare: in the history.
    core.set_output_gain(1, -1.0).unwrap();
    let (state_a, image_a, name_a) = (
        core.get_state(),
        r.remote.image(),
        core.get_device_name().unwrap(),
    );
    assert!(core.can_undo());
    let before = recalls(core).len();

    let s = core.ab_start_documents(String::new(), b).unwrap();
    assert!(s.active);
    assert_eq!(s.side, Some(AbSide::A));
    assert_eq!((s.a_name.as_str(), s.b_name.as_str()), ("A", "Bright"));
    assert_eq!(s.recall, None, "a captured A is not recalled");
    assert_eq!(core.ab_state(), s);
    assert!(core.can_undo(), "starting changes nothing");

    let to_b = core.ab_toggle().unwrap();
    assert_eq!(to_b.side, Some(AbSide::B));
    let rec = to_b.recall.clone().unwrap();
    assert!(rec.image);
    assert!(!rec.changes.is_empty());
    assert_eq!(core.get_device_name().unwrap(), "Bee");
    assert_ne!(core.get_state(), state_a);
    assert_ne!(r.remote.image(), image_a);
    assert!(!core.can_undo(), "a toggle ends the undo history");
    assert_eq!(core.ab_state().side, Some(AbSide::B));

    let to_a = core.ab_toggle().unwrap();
    assert_eq!(to_a.side, Some(AbSide::A));
    assert_eq!(core.get_state(), state_a);
    assert_eq!(r.remote.image(), image_a);
    assert_eq!(core.get_device_name().unwrap(), name_a);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state(), state_a);
    assert!(!core.can_undo(), "no undo step for a toggle");
    assert!(!core.can_redo());

    // Two recalls in the event log, B's then A's.
    let log = recalls(core);
    assert_eq!(log.len() - before, 2, "{log:?}");
    assert!(log[before].contains("Bright"), "{}", log[before]);
    assert!(log[before + 1].contains("\"A\""), "{}", log[before + 1]);

    // Keeping the side it is on recalls nothing and ends the compare.
    let kept = core.ab_keep(AbSide::A).unwrap();
    assert!(!kept.active);
    assert_eq!(kept.side, Some(AbSide::A));
    assert_eq!(kept.recall, None);
    assert_eq!(recalls(core).len() - before, 2);
    assert_eq!(core.ab_state(), AbCompare::none());
    assert!(matches!(
        core.ab_toggle(),
        Err(DspiError::InvalidArgument { .. })
    ));
}

/// `ab_diff` is P1DF's diff for the same pair, A to B.
#[test]
fn wp_p1ab_diff_matches_the_document_diff() {
    let r = rig();
    let core = &r.core;
    let (a, b) = a_and_b(core);
    assert!(matches!(
        core.ab_diff(),
        Err(DspiError::InvalidArgument { .. })
    ));
    core.ab_start_documents(a.clone(), b.clone()).unwrap();
    let diff = core.ab_diff().unwrap();
    assert_eq!(diff, crate::files::config_diff(a, b));
    let v = json_of(&diff);
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["count"].as_u64().unwrap() >= 4, "{v}");
    let rows = v["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["after"].as_str().unwrap().contains("2500")),
        "{v}"
    );
    // The same after a toggle: the diff is A to B, not the device's side.
    core.ab_toggle().unwrap();
    assert_eq!(core.ab_diff().unwrap(), diff);
}

/// A compare ends cleanly on a disconnect: a closed connection, a device
/// lost under a toggle, and a new connection each end it; nothing is sent
/// or replayed.
#[test]
fn wp_p1ab_a_compare_ends_on_a_disconnect() {
    // Lost under a toggle.
    let r = rig();
    let core = &r.core;
    let (_, b) = a_and_b(core);
    core.ab_start_documents(String::new(), b.clone()).unwrap();
    let image = r.remote.image();
    let sent = r.outs.load(Ordering::SeqCst);
    let logged = recalls(core).len();
    r.gone.store(true, Ordering::SeqCst);
    assert_eq!(core.ab_toggle().unwrap_err(), DspiError::Disconnected);
    assert_eq!(
        r.outs.load(Ordering::SeqCst),
        sent,
        "nothing reached the device"
    );
    assert!(!core.ab_state().active, "the compare ended");
    r.gone.store(false, Ordering::SeqCst);
    core.fetch_status().ok();
    assert!(matches!(
        core.ab_toggle(),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(r.remote.image(), image, "nothing replayed");
    assert_eq!(r.outs.load(Ordering::SeqCst), sent);
    assert_eq!(recalls(core).len(), logged);

    // A closed connection.
    core.ab_start_documents(String::new(), b.clone()).unwrap();
    assert!(core.ab_state().active);
    core.disconnect();
    assert_eq!(core.ab_state(), AbCompare::none());
    assert!(matches!(
        core.ab_toggle(),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.ab_keep(AbSide::B),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        core.ab_diff(),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(
        core.ab_start_documents(String::new(), b.clone())
            .unwrap_err(),
        DspiError::Disconnected
    );

    // A new connection does not inherit one.
    let r2 = rig();
    r2.core.ab_start_documents(String::new(), b).unwrap();
    let dev = FakeDevice::demo();
    r2.core
        .attach_probed(Box::new(dev), "P1AB-OTHER".into(), 0)
        .unwrap();
    assert!(!r2.core.ab_state().active);
}

/// From the store: a given A is recalled at the start; keeping the other
/// side recalls it and ends the compare.
#[test]
fn wp_p1ab_start_from_the_store_and_keep() {
    let r = rig();
    let core = &r.core;
    core.snapshot_set_dir(store_dir("store").to_string_lossy().into_owned())
        .unwrap();
    let a = core
        .snapshot_capture("Lugn ☾".into(), String::new())
        .unwrap();
    let state_a = core.get_state();
    change(core);
    let b = core
        .snapshot_capture("Bright".into(), String::new())
        .unwrap();
    let state_b = core.get_state();
    assert!(matches!(
        core.ab_start("nope".into(), b.id.clone()),
        Err(DspiError::InvalidArgument { .. })
    ));

    // The device is on B; starting with A given puts it on A.
    let s = core.ab_start(a.id.clone(), b.id.clone()).unwrap();
    assert_eq!(s.side, Some(AbSide::A));
    assert_eq!(s.a_name, "Lugn ☾");
    assert!(s.recall.is_some());
    assert_eq!(core.get_state(), state_a);

    let kept = core.ab_keep(AbSide::B).unwrap();
    assert!(!kept.active);
    assert_eq!(kept.side, Some(AbSide::B));
    assert!(kept.recall.is_some());
    assert_eq!(core.get_state(), state_b);
    assert_eq!(core.get_device_name().unwrap(), "Bee");
    assert!(!core.ab_state().active);
}

/// Refusals: no device, a document that is not a snapshot, and A and B of
/// another shape; nothing is sent and no compare starts.
#[test]
fn wp_p1ab_refusals_send_nothing() {
    let r = rig();
    let core = &r.core;
    let (a, b) = a_and_b(core);
    let sent = r.outs.load(Ordering::SeqCst);

    // A preset document is not a snapshot.
    let preset = core.config_export("p".into(), "t".into(), 0).unwrap();
    let e = core
        .ab_start_documents(String::new(), preset.clone())
        .unwrap_err();
    assert!(e.to_string().contains("not a snapshot"), "{e}");
    let e = core.ab_start_documents(preset, b.clone()).unwrap_err();
    assert!(e.to_string().contains("not a snapshot"), "{e}");
    assert!(core.ab_start_documents(String::new(), "{".into()).is_err());

    // Another channel count.
    let mut other = json_of(&b);
    other["meta"]["outputChannelCount"] = Value::from(2);
    let e = core
        .ab_start_documents(a.clone(), other.to_string())
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e}");
    let mut other = json_of(&b);
    other["meta"]["platform"] = Value::from("RP2040");
    let mut a_other = json_of(&a);
    a_other["meta"]["platform"] = Value::from("RP2350");
    assert!(
        core.ab_start_documents(a_other.to_string(), other.to_string())
            .is_err()
    );

    assert_eq!(r.outs.load(Ordering::SeqCst), sent);
    assert!(!core.ab_state().active);

    // A side's key round-trips.
    for side in [AbSide::A, AbSide::B] {
        assert_eq!(AbSide::from_key(side.key()), Some(side));
        assert_eq!(side.other().other(), side);
    }
    assert_eq!(AbSide::from_key(" B "), Some(AbSide::B));
    assert_eq!(AbSide::from_key("c"), None);
}

/// Round 2: an edit on B survives two toggles (A/B as in a DAW), each
/// toggle is one recall in the event log, the diff follows the edit, and
/// keeping the live side keeps the edit with nothing sent.
#[test]
fn wp_p1ab_an_edit_on_b_survives_two_toggles() {
    let r = rig();
    let core = &r.core;
    let (_, b) = a_and_b(core);
    let state_a = core.get_state();
    let before = recalls(core).len();
    core.ab_start_documents(String::new(), b).unwrap();
    let diff_before = json_of(&core.ab_diff().unwrap())["count"].as_u64().unwrap();

    core.ab_toggle().unwrap();
    // Tweak B, in and outside the image.
    core.set_output_gain(1, -7.5).unwrap();
    core.set_device_name("Bee tweaked".into()).unwrap();
    let state_b = core.get_state();
    let image_b = r.remote.image();

    let to_a = core.ab_toggle().unwrap();
    assert_eq!(to_a.side, Some(AbSide::A));
    assert_eq!(core.get_state(), state_a);
    let to_b = core.ab_toggle().unwrap();
    assert_eq!(to_b.side, Some(AbSide::B));
    assert_eq!(core.get_state(), state_b, "B kept its tweak");
    assert_eq!(r.remote.image(), image_b);
    assert_eq!(core.get_device_name().unwrap(), "Bee tweaked");
    assert_eq!(to_b.b_name, "Bright", "B keeps its name");

    // One recall per toggle, and only those.
    assert_eq!(recalls(core).len() - before, 3, "{:?}", recalls(core));
    // The diff is between the sides as last left: one more row.
    let diff_now = json_of(&core.ab_diff().unwrap())["count"].as_u64().unwrap();
    assert_eq!(diff_now, diff_before + 1);

    // An edit on B, then keep B: the live state stays, nothing is sent.
    core.set_output_gain(2, -3.5).unwrap();
    let live = core.get_state();
    let sent = r.outs.load(Ordering::SeqCst);
    let kept = core.ab_keep(AbSide::B).unwrap();
    assert_eq!(kept.recall, None);
    assert!(!kept.active);
    assert_eq!(core.get_state(), live);
    assert_eq!(r.outs.load(Ordering::SeqCst), sent);
    assert_eq!(recalls(core).len() - before, 3);
}
