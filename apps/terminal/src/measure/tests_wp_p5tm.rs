//! WP P5TM tests: `uconsole measure` against the demo device and its
//! simulated microphone (P5DEMO), in real time on the core's clock. Nothing
//! plays anywhere: the room is arithmetic. Stores live beside the test
//! binary (`pro::tests::scratch`), never on tmpfs.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError, GuardPhase, GuardedApply, MeasureTuneKeep, MeasureTuneStage};
use dspi_tui::shell::connect::Target;

use super::*;
use crate::pro::tests::{demo, scratch};

/// P5CAL's hand-written fixtures.
const UMIK1: &str =
    include_str!("../../../../crates/dspi-measure/testdata/p5/cal/umik1_7000001.txt");
const TARGET_REW: &str =
    include_str!("../../../../crates/dspi-measure/testdata/p5/cal/target_rew.txt");

/// A demo core with this test's stores, the measurement folder named.
fn rig(name: &str) -> (Arc<DspiCore>, PathBuf) {
    let (core, dir) = demo(&format!("p5tm_{name}"));
    core.measure_set_dir(dir.join("measurements").display().to_string())
        .unwrap();
    (core, dir)
}

/// One measurement of the demo device with the simulated microphone, made
/// once through `measure run` (1 s sweeps) and shared: every demo device
/// has the same serial and the same settings.
fn demo_measurement() -> &'static str {
    static DOC: OnceLock<String> = OnceLock::new();
    DOC.get_or_init(|| {
        let (core, _dir) = rig("shared_run");
        let a = RunArgs {
            sweep_ms: 1_000,
            name: "P5TM demo".into(),
            ..RunArgs::default()
        };
        assert_eq!(measure_run(&core, &a, false, false), exit::OK);
        let docs = core
            .measure_docs("measurement".into(), String::new())
            .unwrap();
        assert_eq!(docs.len(), 1, "{docs:?}");
        assert_eq!(docs[0].name, "P5TM demo");
        core.measure_doc_get(docs[0].id.clone()).unwrap()
    })
}

/// `rig` with the shared measurement filed; its id.
fn rig_with_measurement(name: &str) -> (Arc<DspiCore>, PathBuf, String) {
    let (core, dir) = rig(name);
    let info = core.measure_doc_put(demo_measurement().to_owned()).unwrap();
    (core, dir, info.id)
}

/// What the device holds that a tune changes: every output's gain and every
/// channel's PEQ.
fn settings(core: &DspiCore) -> String {
    let s = core.get_state();
    format!("{:?} {:?}", s.output_gain_db, s.filters)
}

/// Lines for the Keep prompt.
fn answers() -> (Sender<String>, std::sync::mpsc::Receiver<String>) {
    channel()
}

fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_p5tm_arguments_are_checked() {
    assert_eq!(guard_seconds("5"), Ok(5));
    assert_eq!(guard_seconds("590s"), Ok(590));
    assert!(guard_seconds("4").is_err() && guard_seconds("591").is_err());
    assert!(guard_seconds("soon").is_err());
    assert_eq!(output_number("1"), Ok(0));
    assert!(output_number("0").is_err() && output_number("x").is_err());
    assert_eq!(output_list("1,3"), Ok(vec![0, 2]));
    assert!(output_list("1,,2").is_err());
    assert_eq!(file_name("/a/b/umik 90deg.txt"), "umik 90deg.txt");
    assert_eq!(run(&Target::Demo, &["measure", "--help"], false), exit::OK);
    assert_eq!(run(&Target::Demo, &["measure"], false), exit::USAGE);
    assert_eq!(
        run(&Target::Demo, &["measure", "frobnicate"], false),
        exit::USAGE
    );
    assert_eq!(
        run(
            &Target::Demo,
            &["measure", "tune", "x", "--gaurd", "9"],
            false
        ),
        exit::USAGE
    );
    assert_eq!(
        run(&Target::Demo, &["measure", "export", "x"], false),
        exit::USAGE,
        "--output is required"
    );
    assert!(help().contains("measure tune ID [--dry-run] [--guard SECONDS]"));
    assert!(help().contains("--guard 5 to 590"));
    assert_eq!(when(0), "(no date)");
    assert_eq!(when(1_795_057_562_000), "2026-11-19 03:06 UTC");
}

#[test]
fn wp_p5tm_a_mic_and_a_target_are_imported_and_listed() {
    let dir = scratch("p5tm_import");
    // No device: this computer's documents need none.
    let core = pro::prepare(&Stores::under(&dir)).unwrap();
    name_dir(&core, Some(&dir.join("measurements"))).unwrap();
    let mic =
        dspi_core::mic_import(String::new(), "umik1_7000001.txt".into(), UMIK1.into()).unwrap();
    assert_eq!(file_doc(&core, mic, false, false), exit::OK);
    let target = dspi_core::target_import(
        "Room target".into(),
        TARGET_REW.into(),
        "speakers".into(),
        String::new(),
    )
    .unwrap();
    assert_eq!(file_doc(&core, target, false, true), exit::OK);
    let mics = core.measure_docs("mic".into(), String::new()).unwrap();
    assert_eq!(mics.len(), 1, "{mics:?}");
    assert!(mics[0].name.contains("7000001"), "{mics:?}");
    assert_eq!(list(&core, "mic", false), exit::OK);
    assert_eq!(list(&core, "", true), exit::OK);
    assert_eq!(list(&core, "nonsense", false), exit::USAGE);
    // By id and by name; a missing one is a usage error.
    assert_eq!(show(&core, &mics[0].id, false), exit::OK);
    assert_eq!(show(&core, "Room target", false), exit::OK);
    assert_eq!(show(&core, "Room target", true), exit::OK);
    assert_eq!(show(&core, "no such", false), exit::USAGE);
    let (id, text) = resolve(&core, &mics[0].id).unwrap();
    assert_eq!(id, mics[0].id);
    assert!(text.contains("\"kind\": \"mic\"") || text.contains("\"kind\":\"mic\""));
    // A file is a document too.
    let file = dir.join("mic.json");
    std::fs::write(&file, &text).unwrap();
    assert_eq!(show(&core, file.to_str().unwrap(), false), exit::OK);
    // Garbage is refused in words, nothing filed.
    let e = dspi_core::mic_import(String::new(), "x.txt".into(), "\u{0}\u{1}binary".into())
        .unwrap_err();
    assert_eq!(pro::error_exit(&e), exit::USAGE);
    assert_eq!(
        core.measure_docs(String::new(), String::new())
            .unwrap()
            .len(),
        2
    );
    // A microphone has no measured outputs to export.
    assert_eq!(export(&core, &mics[0].id, 0, "-", false), exit::USAGE);
}

#[test]
fn wp_p5tm_a_demo_measurement_is_shown_and_exported() {
    let (core, dir, id) = rig_with_measurement("show");
    let docs = core
        .measure_docs("measurement".into(), String::new())
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert!(!docs[0].serial.is_empty());
    assert_eq!(list(&core, "measurement", false), exit::OK);
    assert_eq!(show(&core, &id, false), exit::OK);
    assert_eq!(show(&core, "P5TM demo", false), exit::OK);
    let file = dir.join("out1.txt");
    assert_eq!(
        export(&core, &id, 0, file.to_str().unwrap(), false),
        exit::OK
    );
    let rew = std::fs::read_to_string(&file).unwrap();
    assert!(rew.contains("P5TM demo"), "{}", &rew[..rew.len().min(400)]);
    // REW's columns: frequency and dB (and phase) per line after the header.
    let rows = rew
        .lines()
        .filter(|l| !l.starts_with('*') && !l.trim().is_empty())
        .filter(|l| {
            l.split_whitespace()
                .take(2)
                .all(|c| c.parse::<f64>().is_ok())
        })
        .count();
    assert!(rows > 100, "{rows} rows");
    assert_eq!(export(&core, &id, 0, "-", true), exit::OK);
    // An output it does not hold.
    assert_eq!(export(&core, &id, 9, "-", false), exit::USAGE);
}

#[test]
fn wp_p5tm_a_tune_is_printed_applied_and_reverted() {
    let (core, _dir, id) = rig_with_measurement("revert");
    let before = settings(&core);
    let (tx, rx) = answers();
    // --dry-run: printed, nothing written, nothing waits.
    let dry = TuneArgs {
        dry_run: true,
        guard_s: 0,
    };
    assert_eq!(tune(&core, &id, &dry, &rx, false, false), exit::OK);
    assert_eq!(settings(&core), before);
    assert_eq!(core.measure_tune_state().stage, MeasureTuneStage::Idle);
    assert_eq!(core.apply_status().phase, GuardPhase::Idle);
    // Applied with --guard 45, then reverted from the prompt.
    let c = Arc::clone(&core);
    let person = std::thread::spawn(move || {
        wait_for("the guard", 30, || {
            c.measure_tune_state().stage == MeasureTuneStage::Verifying
        });
        let g = c.apply_status();
        assert_eq!(g.timeout_ms, 45_000, "{g:?}");
        assert_eq!(g.label, "Auto-tune");
        assert_ne!(settings(&c), before, "the tune is written");
        tx.send("revert".into()).unwrap();
        before
    });
    let a = TuneArgs {
        dry_run: false,
        guard_s: 45,
    };
    assert_eq!(tune(&core, &id, &a, &rx, false, false), exit::REJECTED);
    let before = person.join().unwrap();
    assert_eq!(settings(&core), before, "every value back");
    assert_eq!(core.measure_tune_state().stage, MeasureTuneStage::Reverted);
    assert_eq!(core.apply_status().phase, GuardPhase::Reverted);
    assert!(!core.measure_state().active, "the verify stopped");
}

#[test]
fn wp_p5tm_a_verified_tune_is_kept_with_its_snapshots() {
    let (core, _dir, id) = rig_with_measurement("keep");
    let before = settings(&core);
    let (tx, rx) = answers();
    let c = Arc::clone(&core);
    let person = std::thread::spawn(move || {
        wait_for("the verify", 120, || {
            c.measure_tune_state().verify.is_some()
        });
        let st = c.measure_tune_state();
        assert!(st.verify.as_ref().unwrap().passed, "{:?}", st.verify);
        // The guard's length is the core's: the planned verify plus a margin.
        assert!(st.guard.timeout_ms >= 120_000, "{:?}", st.guard);
        assert_ne!(st.guard.timeout_ms, 30_000);
        tx.send("keep".into()).unwrap();
    });
    let a = TuneArgs::default();
    assert_eq!(tune(&core, &id, &a, &rx, false, false), exit::OK);
    person.join().unwrap();
    assert_ne!(settings(&core), before);
    assert_eq!(core.measure_tune_state().stage, MeasureTuneStage::Kept);
    // The "before" and "after" snapshots are filed, the measurement has the
    // tune's row.
    let snaps = core.snapshot_list(String::new());
    assert!(
        snaps.iter().any(|s| s.name.starts_with("Before auto-tune"))
            && snaps.iter().any(|s| s.name.starts_with("After auto-tune")),
        "{snaps:?}"
    );
    let doc = core.measure_doc_get(id.clone()).unwrap();
    assert!(doc.contains("tunes"), "the measurement's tune row");
    assert_eq!(show(&core, &id, false), exit::OK);
    // One undo puts every value back.
    core.undo().unwrap();
    assert_eq!(settings(&core), before);
}

// A failed verify on the real core needs a summed run (the core's
// `passed` judges the summed runs and sums only), which the demo's two
// full-range outputs never have: the Keep prompt's rule is tested on a
// stand-in below, the core's gate by P5TRUST's own tests.

/// A stand-in for the core: a failed verify, so the first Keep is refused;
/// the guard times out after `ticks` looks.
struct Unverified {
    keeps: Cell<u32>,
    ticks: Cell<u32>,
    timeout_after: u32,
    reverted: Cell<bool>,
    kept: Cell<bool>,
    seen_keeps: RefCell<Vec<u32>>,
}

impl Unverified {
    fn new(timeout_after: u32) -> Self {
        Self {
            keeps: Cell::new(0),
            ticks: Cell::new(0),
            timeout_after,
            reverted: Cell::new(false),
            kept: Cell::new(false),
            seen_keeps: RefCell::new(Vec::new()),
        }
    }
}

impl TuneDriver for Unverified {
    fn tick(&self) -> MeasureTuneState {
        self.ticks.set(self.ticks.get() + 1);
        let mut st = MeasureTuneState {
            stage: MeasureTuneStage::Verified,
            ..MeasureTuneState::default()
        };
        st.guard.phase = GuardPhase::Waiting;
        st.guard.label = "Auto-tune".into();
        st.guard.remaining_ms = 60_000;
        if self.kept.get() {
            st.stage = MeasureTuneStage::Kept;
            st.guard.phase = GuardPhase::Kept;
        } else if self.reverted.get() || self.ticks.get() > self.timeout_after {
            st.stage = MeasureTuneStage::Reverted;
            st.guard.phase = GuardPhase::Reverted;
        }
        st
    }
    fn keep(&self) -> Result<MeasureTuneKeep, DspiError> {
        self.keeps.set(self.keeps.get() + 1);
        self.seen_keeps.borrow_mut().push(self.ticks.get());
        if self.keeps.get() == 1 {
            return Err(DspiError::InvalidArgument {
                msg: format!("{KEEP_UNVERIFIED}: the verify did not confirm the tune"),
            });
        }
        self.kept.set(true);
        let mut k = MeasureTuneKeep::default();
        k.guard.phase = GuardPhase::Kept;
        Ok(k)
    }
    fn revert(&self) -> Result<GuardedApply, DspiError> {
        self.reverted.set(true);
        Ok(GuardedApply::default())
    }
}

#[test]
fn wp_p5tm_a_keep_typed_ahead_is_never_the_confirmation() {
    // Two keeps typed at once and nothing after: one Keep reaches the core,
    // and it is refused; the second line is dropped, and the guard runs out.
    let d = Unverified::new(40);
    let (tx, rx) = answers();
    tx.send("keep".into()).unwrap();
    tx.send("keep".into()).unwrap();
    let (st, keep) = keep_or_revert(&d, &rx, false, false);
    assert_eq!(d.keeps.get(), 1, "never kept without being asked again");
    assert!(keep.is_none());
    assert_eq!(st.guard.phase, GuardPhase::Reverted);
    assert_eq!(report(&st, None, false), exit::REJECTED);
    drop(tx);
}

#[test]
fn wp_p5tm_the_second_keep_after_the_question_keeps() {
    let drv = Unverified::new(10_000);
    let (tx, rx) = answers();
    tx.send("keep".into()).unwrap();
    let sender = std::thread::spawn(move || {
        // The person reads the question, then answers.
        std::thread::sleep(Duration::from_millis(400));
        tx.send("keep".into()).unwrap();
        tx
    });
    let (st, keep) = keep_or_revert(&drv, &rx, false, false);
    let _tx = sender.join().unwrap();
    assert_eq!(drv.keeps.get(), 2);
    let k = keep.unwrap();
    assert_eq!(k.guard.phase, GuardPhase::Kept);
    assert_eq!(st.stage, MeasureTuneStage::Kept);
    assert_eq!(report(&st, Some(&k), false), exit::OK);
    // Revert and other words.
    let drv = Unverified::new(10_000);
    let (tx, rx) = answers();
    tx.send("maybe".into()).unwrap();
    tx.send("r".into()).unwrap();
    let (st, keep) = keep_or_revert(&drv, &rx, false, false);
    assert!(keep.is_none() && drv.reverted.get());
    assert_eq!(st.guard.phase, GuardPhase::Reverted);
    drop(tx);
}

#[test]
fn wp_p5tm_read_only_and_limits_refusals_exit_3_with_nothing_kept() {
    // ReadOnly: the client read-only mode refuses the apply.
    let (core, _dir, id) = rig_with_measurement("read_only");
    // Ready to measure but for the read-only mode: that is the refusal.
    core.set_user_volume(-20.0).unwrap();
    let before = settings(&core);
    core.set_client_read_only(true).unwrap();
    let (_tx, rx) = answers();
    let a = TuneArgs::default();
    assert_eq!(tune(&core, &id, &a, &rx, false, false), exit::REJECTED);
    assert_eq!(settings(&core), before);
    let text = core.measure_doc_get(id).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let p = core
        .measure_tune_propose(
            text,
            layout_for(&core, &v).unwrap(),
            MeasureTuneOptions::default(),
        )
        .unwrap();
    let e = core.measure_tune_apply(p.id, 0).unwrap_err();
    assert!(matches!(e, DspiError::ReadOnly), "{e:?}");
    assert_eq!(refused(&e, false), exit::REJECTED);
    assert_eq!(settings(&core), before);

    // LimitExceeded: a profile that came after the proposal refuses its
    // boost; the guard puts back what was written.
    let (core, _dir, id) = rig_with_measurement("limits");
    let before = settings(&core);
    let text = core.measure_doc_get(id).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let layout = layout_for(&core, &v).unwrap();
    let p = core
        .measure_tune_propose(text, layout, MeasureTuneOptions::default())
        .unwrap();
    assert!(
        p.writes
            .iter()
            .any(|w| w.params.iter().any(|q| q.path == "eq.gain"
                && matches!(q.value, dspi_core::ParamValue::Float(g) if g > 0.5))),
        "a boost to refuse: {:#?}",
        p.writes
    );
    let profile = dspi_session::limits::LimitsProfile {
        max_peq_boost_db: Some(0.0),
        ..dspi_session::limits::LimitsProfile::default()
    };
    core.limits_apply_profile(profile.to_document()).unwrap();
    let (_tx, rx) = answers();
    assert_eq!(
        apply_and_keep(&core, &p, 0, false, &rx, false, false),
        exit::REJECTED
    );
    assert_eq!(settings(&core), before, "nothing kept");
    assert_ne!(core.apply_status().phase, GuardPhase::Waiting);
    assert_eq!(
        pro::error_exit(&DspiError::LimitExceeded {
            code: "peq_boost".into(),
            msg: "x".into(),
        }),
        exit::REJECTED
    );
}

#[test]
fn wp_p5tm_a_viewer_through_the_hub_is_denied_exit_3() {
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        pin_role: "viewer".into(),
        ..HubConfig::bridge()
    })
    .unwrap();
    let dir = scratch("p5tm_viewer");
    let core = pro::prepare(&Stores::under(&dir)).unwrap();
    name_dir(&core, Some(&dir.join("measurements"))).unwrap();
    core.link_connect(
        hub.url(),
        "p5tm".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core.link_pair("123456".into()).unwrap();
    wait_for("the hub's device", 8, || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial.clone()).unwrap();
    let _ = core.fetch_all();
    // The demo measurement, as if made of the hub's device.
    let mut v: Value = serde_json::from_str(demo_measurement()).unwrap();
    v["measurement"]["serial"] = Value::String(d.serial.clone());
    let before = hub.log().len();
    let (_tx, rx) = answers();
    let a = TuneArgs::default();
    assert_eq!(
        tune(&core, &v.to_string(), &a, &rx, false, false),
        exit::REJECTED
    );
    let writes = hub.log().iter().skip(before).filter(|e| e.1 == 0).count();
    assert_eq!(writes, 0, "nothing sent");
    assert_eq!(pro::error_exit(&DspiError::Denied), exit::REJECTED);
    core.link_close();
}

#[test]
fn wp_p5tm_run_measures_only_the_demo() {
    // A core with no device: refused in words, as a usage error.
    let dir = scratch("p5tm_no_demo");
    let core = pro::prepare(&Stores::under(&dir)).unwrap();
    assert_eq!(
        measure_run(&core, &RunArgs::default(), false, false),
        exit::USAGE
    );
    // The demo's: filed with the simulated microphone (the shared run).
    let doc: Value = serde_json::from_str(demo_measurement()).unwrap();
    assert_eq!(doc["measurement"]["capture"]["timingReliable"], true);
    let outputs = doc["measurement"]["outputs"].as_array().unwrap();
    assert!(!outputs.is_empty());
}
