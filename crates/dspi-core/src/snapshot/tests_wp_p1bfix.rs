//! WP P1BFIX tests (review p1b's probes, turned round to assert the fixed
//! behaviour): F1, a recall takes only snapshots, so a plain document cannot
//! skip `config_import`'s token; F4, a recall whose image is refused says
//! what it wrote and what it did not, in the event log and the error.
//!
//! Stores live under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
    TransportError,
};
use serde_json::Value;

use super::*;
use crate::consts::{IMPORT_HARDWARE, IMPORT_VOLUMES};

/// The demo device; with `refuse_bulk` set the whole-image write stalls (as
/// a hub's refusal or a failed transfer would), and OUT transfers that reach
/// the device are counted.
struct NoBulk {
    dev: FakeDevice,
    outs: Arc<AtomicUsize>,
    refuse_bulk: Arc<AtomicBool>,
}

impl Transport for NoBulk {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.dev.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.dev.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        if self.refuse_bulk.load(Ordering::SeqCst)
            && (o == op::REQ_SET_ALL_PARAMS || o == op::REQ_SET_ALL_PARAMS_CHUNK)
        {
            return Err(TransportError::Stalled { opcode: o });
        }
        self.outs.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(o, v, d)
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

fn rig() -> (Arc<DspiCore>, FakeRemote, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let outs = Arc::new(AtomicUsize::new(0));
    let refuse = Arc::new(AtomicBool::new(false));
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(NoBulk {
            dev,
            outs: Arc::clone(&outs),
            refuse_bulk: Arc::clone(&refuse),
        }),
        "P1BFIX-DEMO".into(),
        0,
    )
    .unwrap();
    core.fetch_all().unwrap();
    (core, remote, outs, refuse)
}

fn j(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1bfix-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn is_not_a_snapshot<T: std::fmt::Debug>(r: &Result<T>) -> bool {
    matches!(r, Err(DspiError::InvalidArgument { msg }) if msg.contains(NOT_A_SNAPSHOT))
}

/// F1: a plain document (a template, a preset) is refused by the document
/// recall with the reason and nothing sent, while `config_import` without
/// its token is refused too: the recall is no way round the import's token.
/// With the token the import goes through.
#[test]
fn wp_p1bfix_recall_document_refuses_a_plain_document() {
    let (core, _remote, outs, _) = rig();
    core.set_output_gain(0, -12.0).unwrap();
    let template = core.save_as_template("T".into(), "v".into(), 0).unwrap();
    assert_eq!(j(&template)["meta"]["kind"], "template");
    let mut preset = j(&template);
    preset["meta"]["kind"] = "preset".into();
    let preset = preset.to_string();

    core.set_output_gain(0, -3.0).unwrap();
    let r = j(&core.config_import(template.clone(), IMPORT_VOLUMES | IMPORT_HARDWARE));
    assert_eq!(r["noticeKey"], "confirmation_required", "{r}");

    let n_out = outs.load(Ordering::SeqCst);
    for text in [&template, &preset] {
        let r = core.snapshot_recall_document(text.clone());
        assert!(is_not_a_snapshot(&r), "{r:?}");
    }
    // A document that calls itself a snapshot but has no snapshot block.
    let mut blockless = j(&template);
    blockless["meta"]["kind"] = "snapshot".into();
    let r = core.snapshot_recall_document(blockless.to_string());
    assert!(is_not_a_snapshot(&r), "{r:?}");
    assert_eq!(outs.load(Ordering::SeqCst), n_out, "nothing sent");
    assert_eq!(core.get_state().output_gain_db[0], -3.0, "nothing written");

    // The import's own way, with its token.
    let t = core.confirm_token(crate::ConfirmOp::ConfigImport);
    let r = j(&core.config_import_confirmed(
        template,
        IMPORT_VOLUMES | IMPORT_HARDWARE,
        u32::MAX,
        Some(t),
    ));
    assert_ne!(r["noticeKey"], "confirmation_required", "{r}");
    assert_eq!(core.get_state().output_gain_db[0], -12.0);

    // A real snapshot still needs no token.
    let snap = core
        .snapshot_capture_document("s".into(), String::new())
        .unwrap();
    core.set_output_gain(0, -6.0).unwrap();
    let rec = core.snapshot_recall_document(snap).unwrap();
    assert!(rec.image);
    assert_eq!(core.get_state().output_gain_db[0], -12.0);
}

/// F1 through the store: `snapshot_import` refuses a plain document, and a
/// filed document that is no longer a snapshot (edited by hand) is refused
/// by `snapshot_recall(id)`.
#[test]
fn wp_p1bfix_the_store_takes_and_recalls_only_snapshots() {
    let (core, _remote, outs, _) = rig();
    let root = store_dir("store");
    core.snapshot_set_dir(root.to_string_lossy().into_owned())
        .unwrap();
    let template = core.save_as_template("T".into(), "v".into(), 0).unwrap();
    let r = core.snapshot_import(template, "x".into(), String::new());
    assert!(is_not_a_snapshot(&r), "{r:?}");
    assert_eq!(core.snapshot_list(String::new()), vec![]);

    let info = core.snapshot_capture("s".into(), String::new()).unwrap();
    let path = root
        .join(dspi_session::snapshot_store::dir_name("P1BFIX-DEMO"))
        .join(format!("{}.dspipreset", info.id));
    let mut doc = j(&std::fs::read_to_string(&path).unwrap());
    doc["meta"]["kind"] = "preset".into();
    std::fs::write(&path, doc.to_string()).unwrap();
    let n_out = outs.load(Ordering::SeqCst);
    let r = core.snapshot_recall(info.id);
    assert!(is_not_a_snapshot(&r), "{r:?}");
    assert_eq!(outs.load(Ordering::SeqCst), n_out, "nothing sent");
    let _ = std::fs::remove_dir_all(&root);
}

/// F4: the settings outside the image are written first; when the image is
/// then refused, the recall's error and one Recall event say what was
/// written (the device's and a source's name) and that the image was not.
/// The order stays: image last.
#[test]
fn wp_p1bfix_a_refused_image_reports_a_partial_recall() {
    let (core, remote, outs, refuse) = rig();
    core.set_device_name("Before".into()).unwrap();
    core.set_input_source_name(0, "Src-before".into()).unwrap();
    let doc = core
        .snapshot_capture_document("snap".into(), String::new())
        .unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    core.set_device_name("After".into()).unwrap();
    core.set_input_source_name(0, "Src-after".into()).unwrap();
    core.fetch_all().unwrap();
    let image_before = remote.image();
    let n_out = outs.load(Ordering::SeqCst);
    let events = core.event_log_read(String::new(), 0, 1000).entries.len();

    refuse.store(true, Ordering::SeqCst);
    let e = core.snapshot_recall_document(doc).unwrap_err();
    let DspiError::Transfer { msg } = &e else {
        panic!("a transfer error with the words: {e:?}");
    };
    assert!(msg.contains("partly recalled snapshot \"snap\""), "{msg}");
    assert!(
        msg.contains("its image (every setting in it) was not written"),
        "{msg}"
    );
    assert!(
        msg.contains("Device name: \"After\" to \"Before\""),
        "{msg}"
    );
    assert!(msg.contains("Input source 1 name"), "{msg}");

    // What it says is what happened: names written, the image not.
    assert_eq!(core.get_device_name().unwrap(), "Before");
    assert_eq!(core.cached_input_source_name(0).unwrap(), "Src-before");
    assert!(outs.load(Ordering::SeqCst) > n_out);
    assert_eq!(remote.image(), image_before, "the image was not written");

    let log = core.event_log_read(String::new(), 0, 1000).entries;
    let recalls: Vec<_> = log[events..]
        .iter()
        .filter(|e| e.kind == "recall")
        .collect();
    assert_eq!(recalls.len(), 1, "{log:?}");
    assert!(
        recalls[0]
            .text
            .starts_with("Partly recalled snapshot \"snap\""),
        "{}",
        recalls[0].text
    );
    assert!(
        recalls[0].text.contains("not written"),
        "{}",
        recalls[0].text
    );
    assert_eq!(recalls[0].lines.len(), 2, "{:?}", recalls[0].lines);
    assert!(recalls[0].lines[0].starts_with("Device name"));
}

/// F4: a hub's refusal of the image (`Denied`, no words of its own) reaches
/// the caller as `Link` with that code and the partial recall's words.
#[test]
fn wp_p1bfix_a_denied_image_keeps_its_code_and_gains_the_words() {
    let (core, _remote, _outs, _) = rig();
    let plan = Plan {
        name: "n".into(),
        ..Plan::default()
    };
    let written = vec!["Device name: \"a\" to \"b\"".to_owned()];
    match core.partial_recall(&plan, true, &written, DspiError::Denied) {
        DspiError::Link { code, msg } => {
            assert_eq!(code, "denied");
            assert!(msg.contains("Device name"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        core.partial_recall(&plan, true, &[], DspiError::Disconnected),
        DspiError::Disconnected
    ));
    match core.partial_recall(&plan, false, &[], DspiError::Locked) {
        DspiError::Link { code, msg } => {
            assert_eq!(code, "locked");
            assert!(msg.contains("its settings were not written"), "{msg}");
            assert!(msg.contains("nothing else was changed"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
}
