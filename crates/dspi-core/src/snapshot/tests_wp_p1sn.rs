//! WP P1SN tests: capture and recall on the demo device, the store through
//! the API, and the refusals.
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
use serde_json::{Value, json};

use super::*;
use crate::records::FilterParams;

/// The demo device behind a gate: it counts the OUT transfers that reach
/// the device, and with `gone` set every transfer fails as an unplugged
/// device's does, reaching nothing.
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
    core.attach_probed(Box::new(gate), "P1SN-DEMO".into(), 0)
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
    let d = std::env::temp_dir().join("wp-p1sn-core").join(format!(
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

fn latency(core: &DspiCore) -> u8 {
    let l = core.fetch_output_latency().unwrap();
    let f = l.fields.iter().find(|f| f.name == "requested_id").unwrap();
    f.value.parse().unwrap()
}

/// The parts of the control-surface configuration a snapshot keeps.
fn surfaces(core: &DspiCore) -> Map<String, Value> {
    cs_part(&json_of(&core.cs_snapshot()))
}

fn cs(core: &DspiCore, op: Value) {
    let r = json_of(&core.cs_apply(op.to_string()));
    assert_eq!(r["ok"], true, "{op}: {r}");
}

/// Change something in every part a snapshot holds: the image (preamp, an
/// EQ band, a channel name, an output gain, a crosspoint) and outside it
/// (the device's and a source's name, the output latency, a control-surface
/// group, binding name and display page).
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
    core.set_channel_name(1, "Ändrad 🎚".into()).unwrap();
    core.set_output_gain(1, -3.0).unwrap();
    core.set_device_name("Changed".into()).unwrap();
    core.set_input_source_name(0, "Other".into()).unwrap();
    let was = latency(core);
    core.set_output_latency(if was == 3 { 4 } else { 3 })
        .unwrap();
    let s = json_of(&core.cs_snapshot());
    let mut g = s["groups"][0].clone();
    g["name"] = json!("Moved group");
    cs(core, json!({ "op": "group", "index": 0, "group": g }));
    cs(
        core,
        json!({ "op": "name", "index": 0, "name": "Renamed knob" }),
    );
    let mut c = s["display"]["config"].clone();
    c["dwell"] = json!(c["dwell"].as_u64().unwrap() + 5);
    cs(core, json!({ "op": "displayConfig", "config": c }));
}

/// Capture, change every part, recall: every value back, the undo history
/// ended, one recall in the event log with its changes.
#[test]
fn wp_p1sn_capture_change_recall_puts_every_value_back() {
    let r = rig();
    let core = &r.core;
    core.cs_fetch().unwrap();
    let state = core.get_state();
    let (dev_name, src_name, lat, cs_before) = (
        core.cached_device_name().unwrap(),
        core.cached_input_source_name(0).unwrap(),
        latency(core),
        surfaces(core),
    );
    let doc = core
        .snapshot_capture_document("Évening ☾".into(), "note ✓".into())
        .unwrap();
    let v = json_of(&doc);
    assert_eq!(v["meta"]["kind"], "snapshot");
    assert_eq!(v["meta"]["name"], "Évening ☾");
    assert_eq!(v["snapshot"]["note"], "note ✓");
    assert_eq!(v["snapshot"]["wireFormat"], 32);
    assert_eq!(
        v["snapshot"]["image"].as_str().unwrap().len(),
        2 * BULK_SIZE
    );
    assert!(v["snapshot"]["controlSurface"]["bindings"].is_array());
    assert!(v["snapshot"]["outputLatency"].is_u64());
    // Another console reads it as a document.
    assert_eq!(
        json_of(&crate::files::config_inspect(doc.clone()))["kind"],
        "snapshot"
    );

    change_everything(core);
    assert_ne!(core.get_state(), state);
    assert!(core.can_undo());
    let events = core.event_log_read(String::new(), 0, 1000).entries.len();

    let rec = core.snapshot_recall_document(doc).unwrap();
    assert!(rec.image);
    assert_eq!(rec.name, "Évening ☾");
    assert!(rec.notes.is_empty(), "{:?}", rec.notes);
    for want in [
        "Device name",
        "Input source 1 name",
        "Output latency",
        "Control surfaces: 3 items",
    ] {
        assert!(
            rec.changes.iter().any(|c| c.starts_with(want)),
            "{want}: {:?}",
            rec.changes
        );
    }
    assert!(rec.changes.len() > 5, "{:?}", rec.changes);

    // Bulk and non-bulk, in the mirror and on the device.
    assert_eq!(core.get_state(), state);
    assert_eq!(core.cached_device_name().unwrap(), dev_name);
    assert_eq!(core.get_device_name().unwrap(), dev_name);
    assert_eq!(core.get_input_source_name(0).unwrap(), src_name);
    assert_eq!(latency(core), lat);
    core.cs_fetch().unwrap();
    assert_eq!(surfaces(core), cs_before);
    core.fetch_all().unwrap();
    assert_eq!(core.get_state(), state);

    // The history before it is gone.
    assert!(!core.can_undo());
    // One recall, with its changes.
    let log = core.event_log_read(String::new(), 0, 1000).entries;
    let recalls: Vec<_> = log[events..]
        .iter()
        .filter(|e| e.kind == "recall")
        .collect();
    assert_eq!(recalls.len(), 1, "{log:?}");
    assert!(recalls[0].text.contains("Évening ☾"), "{}", recalls[0].text);
    assert_eq!(recalls[0].lines, rec.changes);
    assert_eq!(recalls[0].channel, None);

    // A second recall changes nothing and says so.
    let again = core.snapshot_recall_document(
        core.snapshot_capture_document("x".into(), String::new())
            .unwrap(),
    );
    assert_eq!(again.unwrap().changes, Vec::<String>::new());
}

/// A recall on a dropped connection sends nothing, and nothing is replayed
/// when the device is back.
#[test]
fn wp_p1sn_a_recall_on_a_dropped_connection_sends_nothing() {
    let r = rig();
    let doc = r
        .core
        .snapshot_capture_document("before".into(), String::new())
        .unwrap();
    r.core.set_input_preamp(0, -9.0).unwrap();
    r.core.set_device_name("After".into()).unwrap();
    let changed = r.remote.image();
    let sent = r.outs.load(Ordering::SeqCst);
    r.gone.store(true, Ordering::SeqCst);
    let e = r.core.snapshot_recall_document(doc.clone()).unwrap_err();
    assert_eq!(e, DspiError::Disconnected);
    assert_eq!(
        r.outs.load(Ordering::SeqCst),
        sent,
        "nothing reached the device"
    );
    // Back: nothing was queued.
    r.gone.store(false, Ordering::SeqCst);
    r.core.fetch_status().ok();
    assert_eq!(r.remote.image(), changed);
    assert_eq!(r.outs.load(Ordering::SeqCst), sent);
    assert_eq!(r.core.get_device_name().unwrap(), "After");

    // And with the connection closed: refused before anything.
    r.core.disconnect();
    assert_eq!(
        r.core.snapshot_recall_document(doc).unwrap_err(),
        DspiError::Disconnected
    );
    assert_eq!(r.outs.load(Ordering::SeqCst), sent);
}

/// Another platform, another channel count, another wire format, a damaged
/// image and a read-only device are refused with the reason, before
/// anything is sent; the history stays.
#[test]
fn wp_p1sn_refusals_send_nothing() {
    let r = rig();
    let core = &r.core;
    let doc = json_of(
        &core
            .snapshot_capture_document("a".into(), String::new())
            .unwrap(),
    );
    core.set_input_preamp(0, -2.0).unwrap();
    let sent = r.outs.load(Ordering::SeqCst);

    let mut platform = doc.clone();
    platform["meta"]["platform"] = json!("RP2040");
    let mut outputs = doc.clone();
    outputs["meta"]["outputChannelCount"] = json!(4);
    let mut damaged = doc.clone();
    damaged["snapshot"]["image"] = json!("zz");
    let mut short = doc.clone();
    short["snapshot"]["image"] = json!("2000");
    // Real V31 and V33 images (the demo as those wire formats).
    let image_of = |wire| hex(&FakeDevice::demo_as_wire(wire).unwrap().remote().image());
    let mut older = doc.clone();
    older["snapshot"]["image"] = json!(image_of(31));
    let mut newer = doc.clone();
    newer["snapshot"]["image"] = json!(image_of(33));
    let cases = [
        (platform, "RP2040"),
        (outputs, "4 outputs"),
        (damaged, "damaged"),
        (short, "damaged"),
        (older, "older firmware (wire format 31)"),
        (newer, "newer firmware (wire format 33)"),
    ];
    for (d, why) in cases {
        match core.snapshot_recall_document(d.to_string()) {
            Err(DspiError::InvalidArgument { msg }) => assert!(msg.contains(why), "{why}: {msg}"),
            other => panic!("{why}: {other:?}"),
        }
        assert_eq!(r.outs.load(Ordering::SeqCst), sent, "{why}");
        assert!(core.can_undo(), "{why}");
    }
    assert!(matches!(
        core.snapshot_recall_document("{".into()),
        Err(DspiError::InvalidArgument { .. })
    ));

    // A read-only device (wire format 31) can be captured but not written.
    let old = DspiCore::new();
    old.open_demo_wire(31).unwrap();
    let text = old
        .snapshot_capture_document("old".into(), String::new())
        .unwrap();
    assert!(json_of(&text)["snapshot"].get("image").is_none());
    assert_eq!(
        old.snapshot_recall_document(text).unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(forward_map(31, 32).map(|f| f(vec![1])), None);
    assert_eq!(forward_map(32, 32).map(|f| f(vec![1])), Some(vec![1]));
}

/// The store through the API: capture, list, rename, note, export, import,
/// recall by id, delete; a damaged index never fails a list; names with any
/// Unicode.
#[test]
fn wp_p1sn_the_store_through_the_api() {
    let r = rig();
    let core = &r.core;
    assert!(matches!(
        core.snapshot_capture("x".into(), String::new()),
        Err(DspiError::Io { .. })
    ));
    assert_eq!(core.snapshot_list(String::new()), vec![]);
    let root = store_dir("api");
    core.snapshot_set_dir(root.to_string_lossy().into_owned())
        .unwrap();

    let names = [
        "Ünïcødé 🎛",
        "../../etc/passwd",
        "a\\b/c:d*e?\"<>|",
        "\u{202e}rtl\0nul",
        "",
    ];
    let mut ids = Vec::new();
    for n in names {
        let i = core
            .snapshot_capture(n.into(), format!("note {n}"))
            .unwrap();
        assert_eq!(i.name, n);
        assert_eq!(i.serial, "P1SN-DEMO");
        assert_eq!((i.platform.as_str(), i.output_channels), ("RP2350", 9));
        assert_eq!(i.wire_format, 32);
        ids.push(i.id);
    }
    let list = core.snapshot_list(String::new());
    assert_eq!(list.len(), names.len());
    assert_eq!(core.snapshot_list("P1SN-DEMO".into()), list);
    assert_eq!(core.snapshot_list("nobody".into()), vec![]);
    let long = "ж".repeat(500);
    let renamed = core.snapshot_rename(ids[0].clone(), long.clone()).unwrap();
    assert_eq!(renamed.name.chars().count(), MAX_NAME_CHARS);
    let noted = core
        .snapshot_set_note(ids[0].clone(), "später".into())
        .unwrap();
    assert_eq!(noted.note, "später");
    let exported = core.snapshot_export(ids[0].clone()).unwrap();
    assert_eq!(json_of(&exported)["snapshot"]["note"], "später");

    // Damage the index: torn, then a hand edit. The list still holds all.
    let index = root
        .join(dspi_session::snapshot_store::dir_name("P1SN-DEMO"))
        .join(dspi_session::snapshot_store::INDEX_FILE);
    let good = std::fs::read_to_string(&index).unwrap();
    std::fs::write(&index, &good[..good.len() / 3]).unwrap();
    assert_eq!(core.snapshot_list(String::new()).len(), names.len());
    std::fs::write(&index, r#"{"snapshots":[{"id":"../x"},{"id":7}]}"#).unwrap();
    assert_eq!(core.snapshot_list(String::new()).len(), names.len());

    // Recall by id.
    core.set_input_preamp(1, -11.0).unwrap();
    let before = core.get_state();
    let rec = core.snapshot_recall(ids[1].clone()).unwrap();
    assert!(rec.image);
    assert_ne!(core.get_state(), before);
    assert!(matches!(
        core.snapshot_recall("../../x".into()),
        Err(DspiError::InvalidArgument { .. })
    ));

    // Import: a plain document of another console is not a snapshot
    // (WP P1BFIX, F1: it goes through `config_import` and its token).
    const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
    let refused = core.snapshot_import(MAC.into(), "From the Mac".into(), "imported".into());
    assert!(
        matches!(&refused, Err(DspiError::InvalidArgument { msg }) if msg.contains(NOT_A_SNAPSHOT)),
        "{refused:?}"
    );
    // A snapshot without an image (another wire format's) is filed, and
    // recalled through the import's path.
    let mut bare = json_of(
        &core
            .snapshot_capture_document("bare".into(), String::new())
            .unwrap(),
    );
    bare["snapshot"].as_object_mut().unwrap().remove("image");
    let bare = bare.to_string();
    let imp = core
        .snapshot_import(bare.clone(), "Without image".into(), "imported".into())
        .unwrap();
    assert_eq!(
        (imp.name.as_str(), imp.note.as_str()),
        ("Without image", "imported")
    );
    assert_eq!(
        json_of(&core.snapshot_export(imp.id.clone()).unwrap())["meta"]["kind"],
        "snapshot"
    );
    core.set_input_preamp(1, -4.0).unwrap();
    let rec = core.snapshot_recall(imp.id.clone()).unwrap();
    assert!(!rec.image);
    assert!(!rec.changes.is_empty());
    let every = crate::consts::IMPORT_BY_BLOCK | crate::consts::IMPORT_ALL_BLOCKS;
    let left = json_of(&core.config_preview(bare, every, u32::MAX));
    assert_eq!(left["count"], 0, "{left}");
    assert!(matches!(
        core.snapshot_import("not json".into(), String::new(), String::new()),
        Err(DspiError::InvalidArgument { .. })
    ));

    core.snapshot_delete(ids[2].clone()).unwrap();
    assert_eq!(core.snapshot_list(String::new()).len(), names.len());
    assert!(core.snapshot_delete(ids[2].clone()).is_err());

    // Forgetting the folder.
    core.snapshot_set_dir(String::new()).unwrap();
    assert_eq!(core.snapshot_list(String::new()), vec![]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn wp_p1sn_hex_round_trips() {
    let b: Vec<u8> = (0..=255).collect();
    assert_eq!(unhex(&hex(&b)).unwrap(), b);
    assert_eq!(unhex("abc"), None);
    assert_eq!(unhex("é1"), None);
}
